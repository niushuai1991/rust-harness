//! `harness sessions/diff/apply/models`：会话、差异审阅与模型管理。
use crate::diffview::{self, Answer, ColorChoice};
use harness_contracts::{AppError, AppResult, ModelConfig, SessionEvent, SessionInfo};
use harness_runtime::Runtime;
use serde_json::{Value, json};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;

fn io_err(error: std::io::Error) -> AppError {
    AppError::new("IO", error.to_string())
}

pub const DEFAULT_MODEL_KEY: &str = "cli.default_model";
pub const DEFAULT_EFFORT_KEY: &str = "cli.default_effort";

#[derive(Debug, clap::Args)]
pub struct SessionsArgs {
    #[command(subcommand)]
    pub cmd: Option<SessionsCmd>,
}

#[derive(Debug, clap::Subcommand)]
pub enum SessionsCmd {
    /// 列出全部会话
    List,
    /// 查看会话信息与尾部事件摘要
    Show {
        /// 会话 ID
        session_id: String,
    },
}

#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    /// 工作区 ID
    pub workspace_id: String,
    /// 颜色输出
    #[arg(long, default_value = "auto")]
    pub color: ColorChoice,
}

#[derive(Debug, clap::Args)]
pub struct ApplyArgs {
    /// 工作区 ID
    pub workspace_id: String,
    /// 跳过逐文件确认（自动排除冲突文件）
    #[arg(long)]
    pub all: bool,
    /// 颜色输出
    #[arg(long, default_value = "auto")]
    pub color: ColorChoice,
}

#[derive(Debug, clap::Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub cmd: Option<ModelsCmd>,
}

#[derive(Debug, clap::Subcommand)]
pub enum ModelsCmd {
    /// 列出全部模型
    List,
    /// 测试模型连接
    Test {
        /// 模型 ID
        model_id: String,
    },
    /// 查看、设置或清除 CLI 默认模型
    Default {
        /// 模型 ID（缺省为查看当前值）
        model_id: Option<String>,
        /// 清除默认模型设置
        #[arg(long, conflicts_with = "model_id")]
        clear: bool,
    },
}

pub async fn sessions(runtime: Arc<Runtime>, args: SessionsArgs) -> AppResult<()> {
    match args.cmd.unwrap_or(SessionsCmd::List) {
        SessionsCmd::List => {
            for session in runtime.store.sessions(None).await? {
                println!(
                    "{}\t{}\t{}\t{}",
                    session.id, session.workspace_id, session.title, session.created_at
                );
            }
        }
        SessionsCmd::Show { session_id } => {
            let info = runtime.store.session(&session_id).await?;
            let events = runtime.store.events(&session_id, 0).await?;
            print!("{}", tail_summary(&events, &info));
        }
    }
    Ok(())
}

/// 会话信息与尾部事件摘要（供测试与 Show 输出）。
pub fn tail_summary(events: &[SessionEvent], info: &SessionInfo) -> String {
    let mut out = format!(
        "会话 {}\t工作区 {}\t{}\n事件总数 {}\n尾部事件：\n",
        info.id,
        info.workspace_id,
        info.title,
        events.len()
    );
    for event in events.iter().rev().take(20).rev() {
        out.push_str(&format!(
            "  {}\t{}\t{}\n",
            event.seq,
            event.kind,
            brief(&event.data)
        ));
    }
    out
}

fn brief(data: &Value) -> String {
    let mut text = data.to_string();
    if text.len() > 120 {
        text = text.chars().take(120).collect();
        text.push('…');
    }
    text
}

pub async fn diff(runtime: Arc<Runtime>, args: DiffArgs) -> AppResult<()> {
    let changes = runtime.workspaces.diff(&args.workspace_id)?;
    if changes.is_empty() {
        println!("工作区 {} 无改动", args.workspace_id);
        return Ok(());
    }
    let color = args.color.effective(std::io::stdout().is_terminal());
    let mut out = std::io::stdout().lock();
    diffview::write_listing(&mut out, &changes, color).map_err(io_err)?;
    if changes.iter().any(|change| change.conflict) {
        eprintln!("提示：CONFLICT 文件不参与 apply，需先处理原项目侧改动。");
    }
    Ok(())
}

pub async fn apply(runtime: Arc<Runtime>, args: ApplyArgs) -> AppResult<()> {
    let changes = runtime.workspaces.diff(&args.workspace_id)?;
    if changes.is_empty() {
        println!("工作区 {} 无改动", args.workspace_id);
        return Ok(());
    }
    let color = args.color.effective(std::io::stdout().is_terminal());
    let (selected, skipped, aborted) = diffview::select(&changes, args.all, |change| {
        // 逐项先展示完整 diff 再确认
        let out = std::io::stdout().lock();
        if let Some(text) = diffview::render(change, color) {
            print!("{text}");
        } else {
            print!("{}", diffview::fallback_note(change));
        }
        drop(out);
        eprint!("应用 {} [{}]？[y/n/a/q] ", change.path, change.kind);
        if std::io::stderr().flush().is_err() {
            return Answer::Quit;
        }
        read_answer()
    });
    if aborted {
        eprintln!("已中止，未应用任何改动。");
        return Ok(());
    }
    if !skipped.is_empty() {
        eprintln!(
            "已排除 {} 个冲突文件：{}",
            skipped.len(),
            skipped.join(", ")
        );
    }
    if selected.is_empty() {
        eprintln!("未选择任何改动。");
        return Ok(());
    }
    let applied = runtime
        .workspaces
        .apply_reviewed(&args.workspace_id, &selected)?;
    println!("已应用 {} 项改动到原项目。", applied.len());
    Ok(())
}

fn read_answer() -> Answer {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Answer::Quit,
        Ok(_) => match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Answer::Yes,
            "n" | "no" => Answer::No,
            "a" | "all" => Answer::All,
            _ => Answer::Quit,
        },
    }
}

pub async fn models(runtime: Arc<Runtime>, args: ModelsArgs) -> AppResult<()> {
    match args.cmd.unwrap_or(ModelsCmd::List) {
        ModelsCmd::List => {
            for model in runtime.models.list_models().await? {
                let mode = serde_json::to_string(&model.api_mode)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned();
                let efforts = if model.efforts.is_empty() {
                    "-".to_string()
                } else {
                    model.efforts.join(",")
                };
                let state = if model.enabled { "" } else { "\t停用" };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}{}",
                    model.id,
                    model.provider_id,
                    mode,
                    model.model,
                    efforts,
                    if model.builtin { "内置" } else { "-" },
                    state
                );
            }
        }
        ModelsCmd::Test { model_id } => {
            runtime.models.test_model(&model_id).await?;
            println!("模型 {model_id} 连接成功");
        }
        ModelsCmd::Default { model_id, clear } => {
            if clear {
                runtime.store.set(DEFAULT_MODEL_KEY, &Value::Null).await?;
                println!("已清除 CLI 默认模型。");
                return Ok(());
            }
            match model_id {
                Some(id) => {
                    ensure_model_exists(&runtime.models.list_models().await?, &id)?;
                    runtime.store.set(DEFAULT_MODEL_KEY, &json!(id)).await?;
                    println!("已设置 CLI 默认模型：{id}");
                }
                None => match current_default_model(&runtime).await? {
                    Some(id) => println!("CLI 默认模型：{id}"),
                    None => println!("CLI 默认模型未设置（harness models default <id>）。"),
                },
            }
        }
    }
    Ok(())
}

/// run/chat 的默认模型解析：-m > settings > 报错并列出可用模型。
pub async fn resolve_model_id(
    runtime: &Arc<Runtime>,
    explicit: Option<String>,
) -> AppResult<String> {
    if let Some(id) = explicit {
        return Ok(id);
    }
    if let Some(id) = current_default_model(runtime).await? {
        return Ok(id);
    }
    let models = runtime.models.list_models().await?;
    let listing = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(AppError::new(
        "MODEL_NOT_SELECTED",
        format!("未选择模型：用 -m <id> 或 harness models default <id>；可用模型：{listing}"),
    ))
}

async fn current_default_model(runtime: &Arc<Runtime>) -> AppResult<Option<String>> {
    Ok(runtime
        .store
        .get(DEFAULT_MODEL_KEY)
        .await?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

pub fn ensure_model_exists(models: &[ModelConfig], id: &str) -> AppResult<()> {
    if models.iter().any(|model| model.id == id) {
        Ok(())
    } else {
        Err(AppError::new(
            "MODEL_NOT_FOUND",
            format!("模型 {id} 不存在。可用模型：{}", enabled_id_list(models)),
        ))
    }
}

/// 错误提示用的启用模型 id 清单。
pub fn enabled_id_list(models: &[ModelConfig]) -> String {
    models
        .iter()
        .filter(|model| model.enabled)
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 无效或缺省档位的归一化：回落模型最高档；无档位模型返回 None。
pub fn normalize_effort(model: &ModelConfig, effort: Option<String>) -> Option<String> {
    if model.efforts.is_empty() {
        return None;
    }
    match effort {
        Some(value) if model.efforts.contains(&value) => Some(value),
        _ => model.efforts.last().cloned(),
    }
}

pub async fn current_default_effort(runtime: &Arc<Runtime>) -> AppResult<Option<String>> {
    Ok(runtime
        .store
        .get(DEFAULT_EFFORT_KEY)
        .await?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

/// 未显式指定档位时的回落链：CLI 默认档位 > 模型最高档。
pub async fn fallback_effort(runtime: &Arc<Runtime>, model_id: &str) -> AppResult<Option<String>> {
    let catalog = runtime.models.list_models().await?;
    match catalog.iter().find(|model| model.id == model_id) {
        Some(config) => {
            let stored = current_default_effort(runtime).await?;
            Ok(normalize_effort(config, stored))
        }
        None => Ok(None),
    }
}

/// 会话内切换后静默写入持久化默认（对齐桌面端 `app.default_selection` 的双写语义）；
/// 失败由调用方以警告呈现，不影响切换本身。
pub async fn persist_default_selection(
    runtime: &Arc<Runtime>,
    model_id: &str,
    effort: Option<&str>,
) -> AppResult<()> {
    runtime
        .store
        .set(DEFAULT_MODEL_KEY, &json!(model_id))
        .await?;
    runtime
        .store
        .set(
            DEFAULT_EFFORT_KEY,
            &effort.map(|value| json!(value)).unwrap_or(Value::Null),
        )
        .await
}

pub const DEFAULT_PRESET_KEY: &str = "cli.default_preset";

#[derive(Debug, clap::Args)]
pub struct PresetsArgs {
    #[command(subcommand)]
    pub cmd: Option<PresetsCmd>,
}

#[derive(Debug, clap::Subcommand)]
pub enum PresetsCmd {
    /// 列出全部预设（内置置顶，附可用性标记）
    List,
    /// 保存（upsert）一个预设；未指定的能力字段不覆盖
    Save {
        /// 预设 ID
        id: String,
        /// 显示名称（缺省取 ID）
        #[arg(long)]
        name: Option<String>,
        /// 一句话说明
        #[arg(long)]
        description: Option<String>,
        /// 模型 ID
        #[arg(long)]
        model: Option<String>,
        /// 推理档位（必须在所选模型支持列表内）
        #[arg(long)]
        effort: Option<String>,
        /// 审批策略 ask|auto|never（仅 CLI/JSON-RPC/ACP 生效）
        #[arg(long)]
        approval: Option<String>,
    },
    /// 删除一个用户预设
    Remove {
        /// 预设 ID
        id: String,
    },
    /// 查看、设置或清除 CLI 默认预设（run/chat/ACP 未显式 --preset 时的回落）
    Default {
        /// 预设 ID（缺省为查看当前值）
        preset_id: Option<String>,
        /// 清除默认预设设置
        #[arg(long, conflicts_with = "preset_id")]
        clear: bool,
    },
}

pub async fn presets(runtime: Arc<Runtime>, args: PresetsArgs) -> AppResult<()> {
    match args.cmd.unwrap_or(PresetsCmd::List) {
        PresetsCmd::List => {
            for info in runtime.presets.list().await? {
                let state = match &info.broken {
                    Some(reason) => format!("\t不可用（{reason}）"),
                    None => String::new(),
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}{}",
                    info.preset.id,
                    info.preset.name,
                    info.preset.model_id.as_deref().unwrap_or("-"),
                    info.preset.effort.as_deref().unwrap_or("-"),
                    info.preset.approval.as_deref().unwrap_or("-"),
                    if info.preset.id == harness_runtime::presets::BUILTIN_PRESET_ID {
                        "内置"
                    } else {
                        "-"
                    },
                    state
                );
            }
        }
        PresetsCmd::Save {
            id,
            name,
            description,
            model,
            effort,
            approval,
        } => {
            let saved = runtime
                .presets
                .save(harness_contracts::PresetConfig {
                    id,
                    name: name.unwrap_or_default(),
                    description,
                    model_id: model,
                    effort,
                    approval,
                })
                .await?;
            println!("已保存预设 {}（{}）", saved.id, saved.name);
        }
        PresetsCmd::Remove { id } => {
            runtime.presets.remove(&id).await?;
            println!("已删除预设 {id}");
        }
        PresetsCmd::Default { preset_id, clear } => {
            if clear {
                runtime.store.set(DEFAULT_PRESET_KEY, &Value::Null).await?;
                println!("已清除 CLI 默认预设。");
                return Ok(());
            }
            match preset_id {
                Some(id) => {
                    runtime.presets.resolve(&id).await?;
                    runtime.store.set(DEFAULT_PRESET_KEY, &json!(id)).await?;
                    println!("已设置 CLI 默认预设：{id}");
                }
                None => match current_default_preset(&runtime).await? {
                    Some(id) => println!("CLI 默认预设：{id}"),
                    None => println!("CLI 默认预设未设置（harness presets default <id>）。"),
                },
            }
        }
    }
    Ok(())
}

/// run/chat/ACP 的预设解析：显式 --preset > settings `cli.default_preset` > 无预设。
pub async fn resolve_preset(
    runtime: &Arc<Runtime>,
    explicit: Option<&str>,
) -> AppResult<Option<harness_runtime::presets::ResolvedPreset>> {
    let id = match explicit {
        Some(id) => Some(id.to_owned()),
        None => current_default_preset(runtime).await?,
    };
    match id {
        Some(id) => Ok(Some(runtime.presets.resolve(&id).await?)),
        None => Ok(None),
    }
}

pub(crate) fn preset_model(
    preset: &Option<harness_runtime::presets::ResolvedPreset>,
) -> Option<String> {
    preset
        .as_ref()
        .and_then(|resolved| resolved.model_id.clone())
}

pub(crate) fn preset_effort(
    preset: &Option<harness_runtime::presets::ResolvedPreset>,
) -> Option<String> {
    preset.as_ref().and_then(|resolved| resolved.effort.clone())
}

async fn current_default_preset(runtime: &Arc<Runtime>) -> AppResult<Option<String>> {
    Ok(runtime
        .store
        .get(DEFAULT_PRESET_KEY)
        .await?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str) -> ModelConfig {
        serde_json::from_value(json!({
            "id": id, "name": id, "baseUrl": "http://x", "model": "m",
            "credentialRef": "c", "proxy": null, "providerId": "p",
            "apiMode": "chatCompletions", "maxOutputTokens": 1024
        }))
        .unwrap()
    }

    #[test]
    fn default_model_must_exist_when_setting() {
        let models = vec![model("a"), model("b")];
        assert!(ensure_model_exists(&models, "a").is_ok());
        let error = ensure_model_exists(&models, "missing").unwrap_err();
        assert_eq!(error.code, "MODEL_NOT_FOUND");
        assert!(error.message.contains("可用模型"), "错误应列出可用模型");
    }

    fn model_with_efforts(id: &str, efforts: &[&str]) -> ModelConfig {
        let mut config = model(id);
        config.efforts = efforts.iter().map(|value| value.to_string()).collect();
        config
    }

    #[test]
    fn normalize_effort_falls_back_to_highest_level() {
        let config = model_with_efforts("m", &["low", "high", "max"]);
        // 缺省与无效档位都回落最高档
        assert_eq!(normalize_effort(&config, None).as_deref(), Some("max"));
        assert_eq!(
            normalize_effort(&config, Some("ultra".into())).as_deref(),
            Some("max")
        );
        // 支持的档位原样保留
        assert_eq!(
            normalize_effort(&config, Some("low".into())).as_deref(),
            Some("low")
        );
        // 无档位模型永远为 None
        let bare = model_with_efforts("bare", &[]);
        assert_eq!(normalize_effort(&bare, Some("high".into())), None);
        assert_eq!(normalize_effort(&bare, None), None);
    }

    #[test]
    fn enabled_id_list_skips_disabled_models() {
        let mut disabled = model("off");
        disabled.enabled = false;
        let models = vec![model("on"), disabled];
        assert_eq!(enabled_id_list(&models), "on");
    }
}
