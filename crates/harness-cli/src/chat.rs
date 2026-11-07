//! `harness chat`：终端交互式对话（REPL），复用 run 的单轮执行内核。
use crate::{
    approval::Approval,
    diffview::ColorChoice,
    line_editor::{self, LineResult, ModelRow, PickerContext},
    manage::{self},
    run,
    slash::{ChatCommand, SLASH_COMMANDS, parse_command},
};
use clap::ValueEnum;
use harness_contracts::{AppError, AppResult, ModelConfig};
use harness_runtime::Runtime;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 目标项目目录（默认当前目录）
    #[arg(short = 'C', long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    /// 模型 ID（默认 settings 的 cli.default_model，否则报错列出可用模型）
    #[arg(short = 'm', long)]
    pub model: Option<String>,
    /// 推理档位（必须在所选模型的支持档位内；缺省取 CLI 默认档位，否则模型最高档）
    #[arg(short = 'e', long)]
    pub effort: Option<String>,
    /// 显式指定工作区（与 --session 互斥）
    #[arg(long, conflicts_with = "session")]
    pub workspace: Option<String>,
    /// 续已有会话
    #[arg(long)]
    pub session: Option<String>,
    /// 审批策略；chat 默认 ask
    #[arg(long)]
    pub approval: Option<Approval>,
    /// 能力预设（模型/档位/审批策略的命名组合；显式参数优先于预设字段）
    #[arg(long)]
    pub preset: Option<String>,
    /// 颜色输出
    #[arg(long, default_value = "auto")]
    pub color: ColorChoice,
}

/// `/init` 注入的预置提示词：模型在隔离副本生成 AGENTS.md，经 /diff /apply 审阅同步。
const INIT_PROMPT: &str = "请分析当前项目并在执行根目录生成 AGENTS.md：\n\
1. 项目概述（用途、技术栈、构建/测试/运行命令）\n\
2. 目录结构与关键模块说明\n\
3. 协作约定（代码风格、提交与验证要求）\n\
要求内容精炼并全部使用中文；若已存在 AGENTS.md 则读取后在原基础上修订补全。完成后简述你做了什么。";

/// `/plan` 注入的预置提示词：模型只读探索后制定实施计划并经 plan_write 记录；未经确认不改动文件。
/// 桌面端 `frontend/slashCommands.ts` 中的同名常量以此处为单一事实源。
const PLAN_PROMPT: &str = "请为下面的任务制定实施计划：先阅读相关代码弄清现状，再给出分步骤、可验证的计划，\
并用 plan_write 工具把计划记录下来；计划未经我确认前不要修改任何文件。";

/// `/goal` 注入的预置提示词：模型用 goal_write 记录目标并说明推进方式；未经确认不改动文件。
/// 桌面端 `frontend/slashCommands.ts` 中的同名常量以此处为单一事实源。
const GOAL_PROMPT: &str = "请把下面的内容设定为本会话的目标：用 goal_write 工具记录目标，\
并简要说明你打算如何推进；未经我确认前不要修改任何文件。";

/// 把用户跟在 `/plan`、`/goal` 后的文本拼进预置提示词；格式与桌面端 expandSlashPrompt 保持镜像。
fn slash_prompt(prompt: &str, label: &str, body: &str) -> String {
    format!("{prompt}\n\n{label}：{body}")
}

/// 列出 /resume 展示的会话数量上限。
const RESUME_LIST_LIMIT: usize = 10;

/// 由命令注册表生成帮助文案。
fn help_text() -> String {
    let mut text = String::from("可用命令：\n");
    for spec in SLASH_COMMANDS {
        text.push_str(&format!("  {:<28} {}\n", spec.display(), spec.description));
    }
    text.push_str("其余输入将作为任务消息发送给 Agent。输入 / 可浏览命令补全。");
    text
}

pub async fn execute(runtime: Arc<Runtime>, args: Args) -> AppResult<()> {
    if !std::io::stdin().is_terminal() {
        return Err(AppError::new("TTY", "chat 需要交互式终端（stdin 为 TTY）"));
    }
    // 预设层：显式 --preset > settings `cli.default_preset`；字段级仍以显式参数优先。
    let preset = manage::resolve_preset(&runtime, args.preset.as_deref()).await?;
    let preset_approval = preset
        .as_ref()
        .and_then(|resolved| resolved.approval.as_deref())
        .map(crate::approval::parse_approval)
        .transpose()?;
    let mut policy = args.approval.or(preset_approval).unwrap_or(Approval::Ask);
    let (workspace_id, mut session_id, warning) = run::resolve_session_target(
        &runtime,
        args.cwd.as_deref(),
        args.workspace.as_deref(),
        args.session.as_deref(),
        "chat 会话",
    )
    .await?;
    let mut model_id = manage::resolve_model_id(
        &runtime,
        args.model.clone().or_else(|| manage::preset_model(&preset)),
    )
    .await?;
    // 档位解析：-e/预设显式传入（不支持时由构建报错），否则 CLI 默认档位 > 模型最高档。
    let mut effort = match args
        .effort
        .clone()
        .or_else(|| manage::preset_effort(&preset))
    {
        Some(explicit) => Some(explicit),
        None => manage::fallback_effort(&runtime, &model_id).await?,
    };
    let mut provider = runtime
        .models
        .build_model_with(&model_id, effort.clone())
        .await?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    eprintln!("chat 就绪：模型 {model_id}，会话 {session_id}。/help 查看命令，/quit 退出。");

    let mut history = Vec::new();
    loop {
        // 每轮刷新面板目录快照（不缓存）；读取失败时面板退化为纯命令补全。
        let picker = picker_context(&runtime, &model_id, effort.as_deref()).await;
        let line = match line_editor::read_line("你> ", &mut history, picker) {
            Ok(LineResult::Line(line)) => line,
            Ok(LineResult::Interrupt) | Ok(LineResult::Eof) => break,
            Err(error) => return Err(AppError::new("TTY", error.to_string())),
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let context = run::TurnContext {
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            provider: provider.clone(),
            policy,
            json: false,
            color: ColorChoice::Auto,
        };
        match parse_command(trimmed) {
            ChatCommand::Quit => break,
            ChatCommand::Help => eprintln!("{}", help_text()),
            ChatCommand::Model(None) => match runtime.models.list_models().await {
                Ok(models) => {
                    for model in &models {
                        let marker = if model.id == model_id { " *当前" } else { "" };
                        let efforts = if model.efforts.is_empty() {
                            "-".to_owned()
                        } else {
                            model.efforts.join(",")
                        };
                        eprintln!("{}\t{}\t{}{marker}", model.id, model.model, efforts);
                    }
                }
                Err(error) => eprintln!("读取模型目录失败：{error}"),
            },
            ChatCommand::Model(Some(id)) => {
                // 校验/切档失败只打印错误并继续，不再让手误退出整个会话。
                match prepare_model_switch(&runtime, &effort, id).await {
                    Ok(new_effort) => {
                        match runtime
                            .models
                            .build_model_with(id, new_effort.clone())
                            .await
                        {
                            Ok(new_provider) => {
                                eprintln!(
                                    "── 模型已切换：{model_id} → {id}（档位：{}）──",
                                    new_effort.as_deref().unwrap_or("默认")
                                );
                                model_id = id.to_owned();
                                effort = new_effort;
                                provider = new_provider;
                                // 静默写入持久化默认；失败仅警告，不影响本次切换。
                                if let Err(error) = manage::persist_default_selection(
                                    &runtime,
                                    &model_id,
                                    effort.as_deref(),
                                )
                                .await
                                {
                                    eprintln!("警告：默认模型写入失败：{error}");
                                }
                            }
                            Err(error) => eprintln!("切换失败：{error}"),
                        }
                    }
                    Err(error) => eprintln!("{error}"),
                }
            }
            ChatCommand::Effort(None) => match current_model_efforts(&runtime, &model_id).await {
                Ok(levels) => eprintln!(
                    "当前档位：{}（可用：{}）",
                    effort.as_deref().unwrap_or("默认"),
                    levels.join("、")
                ),
                Err(error) => eprintln!("读取模型目录失败：{error}"),
            },
            ChatCommand::Effort(Some(level)) => {
                match current_model_efforts(&runtime, &model_id).await {
                    Ok(levels) => {
                        match levels
                            .iter()
                            .find(|candidate| candidate.eq_ignore_ascii_case(level))
                        {
                            Some(matched) => {
                                let previous = effort.clone();
                                match runtime
                                    .models
                                    .build_model_with(&model_id, Some(matched.clone()))
                                    .await
                                {
                                    Ok(new_provider) => {
                                        effort = Some(matched.clone());
                                        provider = new_provider;
                                        eprintln!(
                                            "── 档位已切换：{} → {matched} ──",
                                            previous.as_deref().unwrap_or("默认")
                                        );
                                        if let Err(error) = manage::persist_default_selection(
                                            &runtime,
                                            &model_id,
                                            effort.as_deref(),
                                        )
                                        .await
                                        {
                                            eprintln!("警告：默认档位写入失败：{error}");
                                        }
                                    }
                                    Err(error) => eprintln!("切换失败：{error}"),
                                }
                            }
                            None => eprintln!(
                                "档位 {level} 不受当前模型支持。可用：{}",
                                levels.join("、")
                            ),
                        }
                    }
                    Err(error) => eprintln!("读取模型目录失败：{error}"),
                }
            }
            ChatCommand::Status => {
                let info = runtime.workspaces.get(&workspace_id)?;
                eprintln!(
                    "会话    {session_id}\n工作区  {}\n副本    {}\n模型    {model_id}\n档位    {}\n审批    {policy:?}",
                    info.source_path,
                    info.replica_path,
                    effort.as_deref().unwrap_or("默认"),
                );
            }
            ChatCommand::Clear => {
                session_id = runtime
                    .store
                    .create_session(&workspace_id, "新会话")
                    .await?
                    .id;
                eprintln!("已重开会话：{session_id}（原会话保留，可用 /resume 切回）");
            }
            ChatCommand::Compact => {
                eprintln!("正在压缩会话历史……");
                match harness_orchestration::compact_session(
                    runtime.clone(),
                    &session_id,
                    provider.clone(),
                    Default::default(),
                )
                .await
                {
                    Ok(()) => eprintln!("历史已压缩为摘要，后续回合将基于摘要继续。"),
                    Err(error) => eprintln!("压缩失败：{error}"),
                }
            }
            ChatCommand::Init => {
                if let Err(error) = run::turn_once(runtime.clone(), context, INIT_PROMPT).await {
                    eprintln!("回合发起失败：{error}");
                } else {
                    eprintln!("提示：用 /diff 查看 AGENTS.md 改动，/apply 审阅后同步到原项目。");
                }
            }
            ChatCommand::Plan(None) => {
                eprintln!("用法：/plan <任务描述>；发送后模型制定实施计划并经 plan_write 记录。");
            }
            ChatCommand::Plan(Some(task)) => {
                let prompt = slash_prompt(PLAN_PROMPT, "任务", task);
                if let Err(error) = run::turn_once(runtime.clone(), context, &prompt).await {
                    eprintln!("回合发起失败：{error}");
                } else {
                    eprintln!("计划已发起；确认后可直接发送消息开始实施。");
                }
            }
            ChatCommand::Goal(None) => {
                eprintln!("用法：/goal <目标描述>；发送后模型经 goal_write 记录本会话目标。");
            }
            ChatCommand::Goal(Some(objective)) => {
                let prompt = slash_prompt(GOAL_PROMPT, "目标", objective);
                if let Err(error) = run::turn_once(runtime.clone(), context, &prompt).await {
                    eprintln!("回合发起失败：{error}");
                } else {
                    eprintln!("目标已发起；后续回合将围绕该目标推进。");
                }
            }
            ChatCommand::Resume(None) => {
                let sessions = runtime.list_sessions(Some(&workspace_id)).await?;
                let listed: Vec<_> = sessions
                    .iter()
                    .filter(|info| info.parent_id.is_none())
                    .take(RESUME_LIST_LIMIT)
                    .collect();
                for info in &listed {
                    let marker = if info.id == session_id {
                        " *当前"
                    } else {
                        ""
                    };
                    eprintln!("{}\t{}\t{}{marker}", info.id, info.title, info.created_at);
                }
                if listed.is_empty() {
                    eprintln!("当前工作区暂无会话。");
                } else {
                    eprintln!("用法：/resume <id> 切换会话。");
                }
            }
            ChatCommand::Resume(Some(id)) => {
                let sessions = runtime.list_sessions(Some(&workspace_id)).await?;
                match sessions.iter().find(|info| info.id == id) {
                    Some(info) => {
                        session_id = info.id.clone();
                        eprintln!("已切换会话：{session_id}（{}）", info.title);
                    }
                    None => {
                        eprintln!(
                            "会话不存在或不属于当前工作区：{id}；不带参数执行 /resume 可列出。"
                        )
                    }
                }
            }
            ChatCommand::Sessions => {
                manage::sessions(
                    runtime.clone(),
                    manage::SessionsArgs {
                        cmd: Some(manage::SessionsCmd::List),
                    },
                )
                .await?;
            }
            ChatCommand::Diff => {
                manage::diff(
                    runtime.clone(),
                    manage::DiffArgs {
                        workspace_id: workspace_id.clone(),
                        color: ColorChoice::Auto,
                    },
                )
                .await?;
            }
            ChatCommand::Apply => {
                manage::apply(
                    runtime.clone(),
                    manage::ApplyArgs {
                        workspace_id: workspace_id.clone(),
                        all: false,
                        color: ColorChoice::Auto,
                    },
                )
                .await?;
            }
            ChatCommand::Approval(None) => eprintln!("当前审批策略：{policy:?}"),
            ChatCommand::Approval(Some(value)) => {
                let parsed = Approval::from_str(value, true)
                    .map_err(|options| AppError::new("ARGS", format!("可选值：{options}")))?;
                policy = parsed;
                eprintln!("审批策略已切换：{policy:?}");
            }
            ChatCommand::Unknown(command) => {
                eprintln!("未知命令 {command}；/help 查看可用命令。");
            }
            ChatCommand::Message => {
                if let Err(error) = run::turn_once(runtime.clone(), context, trimmed).await {
                    eprintln!("回合发起失败：{error}");
                }
            }
        }
    }
    eprintln!("已退出 chat。");
    Ok(())
}

/// `/model <id>` 的前置校验与档位归一：目标必须存在且启用；
/// 当前档位被新模型支持则保留，否则回落其最高档。
async fn prepare_model_switch(
    runtime: &Arc<Runtime>,
    current_effort: &Option<String>,
    target: &str,
) -> AppResult<Option<String>> {
    let models = runtime.models.list_models().await?;
    let config = models
        .iter()
        .find(|model| model.id == target)
        .ok_or_else(|| {
            AppError::new(
                "MODEL_NOT_FOUND",
                format!(
                    "模型 {target} 不存在。可用模型：{}",
                    manage::enabled_id_list(&models)
                ),
            )
        })?;
    if !config.enabled {
        return Err(AppError::new(
            "MODEL_DISABLED",
            format!("模型 {target} 已停用，无法切换。"),
        ));
    }
    Ok(manage::normalize_effort(config, current_effort.clone()))
}

/// 当前模型支持的档位列表（目录读取失败向上传播，由调用方打印并继续）。
async fn current_model_efforts(runtime: &Arc<Runtime>, model_id: &str) -> AppResult<Vec<String>> {
    Ok(runtime
        .models
        .list_models()
        .await?
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| model.efforts.clone())
        .unwrap_or_default())
}

/// 每轮输入前刷新内联面板上下文（目录不缓存，与桌面端一致）；
/// 目录读取失败时退化为空面板（`/model`、`/effort` 仍可作普通命令补全）。
async fn picker_context(
    runtime: &Arc<Runtime>,
    model_id: &str,
    effort: Option<&str>,
) -> PickerContext {
    let models = runtime.models.list_models().await.unwrap_or_default();
    let providers = runtime.models.list_providers().await.unwrap_or_default();
    let usable = |model: &ModelConfig| {
        model.enabled
            && providers
                .iter()
                .any(|provider| provider.id == model.provider_id && provider.enabled)
    };
    let rows = models
        .iter()
        .filter(|model| usable(model))
        .map(|model| ModelRow {
            id: model.id.clone(),
            label: if model.name.is_empty() {
                model.model.clone()
            } else {
                model.name.clone()
            },
            provider: providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .map(|provider| provider.name.clone())
                .unwrap_or_default(),
            current: model.id == model_id,
        })
        .collect();
    let efforts = models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| model.efforts.clone())
        .unwrap_or_default();
    PickerContext {
        models: rows,
        efforts,
        current_effort: effort.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_lists_every_registered_command() {
        let text = help_text();
        for spec in SLASH_COMMANDS {
            assert!(
                text.contains(&spec.display()),
                "帮助缺少 {}",
                spec.display()
            );
            assert!(
                text.contains(spec.description),
                "帮助缺少描述：{}",
                spec.description
            );
        }
    }

    #[test]
    fn init_prompt_mentions_agents_md() {
        assert!(INIT_PROMPT.contains("AGENTS.md"));
    }

    #[test]
    fn plan_prompt_embeds_task_and_tool_name() {
        let prompt = slash_prompt(PLAN_PROMPT, "任务", "为导出功能制定计划");
        assert!(prompt.contains("plan_write"));
        assert!(prompt.contains("为导出功能制定计划"));
        assert!(prompt.starts_with(PLAN_PROMPT));
    }

    #[test]
    fn goal_prompt_embeds_objective_and_tool_name() {
        let prompt = slash_prompt(GOAL_PROMPT, "目标", "完成导出功能");
        assert!(prompt.contains("goal_write"));
        assert!(prompt.contains("完成导出功能"));
        assert!(prompt.starts_with(GOAL_PROMPT));
    }
}
