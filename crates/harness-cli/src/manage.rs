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
                println!(
                    "{}\t{}\t{}\t{}",
                    model.id, model.provider_id, mode, model.model
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
            format!("模型 {id} 不存在，无法设为默认"),
        ))
    }
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
    }
}
