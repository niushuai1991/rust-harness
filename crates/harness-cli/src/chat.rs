//! `harness chat`：终端交互式对话（REPL），复用 run 的单轮执行内核。
use crate::{
    approval::Approval,
    diffview::ColorChoice,
    line_editor::{self, LineResult},
    manage::{self, ensure_model_exists},
    run,
    slash::{ChatCommand, SLASH_COMMANDS, parse_command},
};
use clap::ValueEnum;
use harness_contracts::{AppError, AppResult};
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
    /// 显式指定工作区（与 --session 互斥）
    #[arg(long, conflicts_with = "session")]
    pub workspace: Option<String>,
    /// 续已有会话
    #[arg(long)]
    pub session: Option<String>,
    /// 审批策略；chat 默认 ask
    #[arg(long)]
    pub approval: Option<Approval>,
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
    let mut policy = args.approval.unwrap_or(Approval::Ask);
    let (workspace_id, mut session_id, warning) = run::resolve_session_target(
        &runtime,
        args.cwd.as_deref(),
        args.workspace.as_deref(),
        args.session.as_deref(),
        "chat 会话",
    )
    .await?;
    let mut model_id = manage::resolve_model_id(&runtime, args.model.clone()).await?;
    let mut provider = runtime.models.build_model(&model_id).await?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    eprintln!("chat 就绪：模型 {model_id}，会话 {session_id}。/help 查看命令，/quit 退出。");

    let mut history = Vec::new();
    loop {
        let line = match line_editor::read_line("你> ", &mut history) {
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
            ChatCommand::Model(None) => {
                for model in runtime.models.list_models().await? {
                    let marker = if model.id == model_id { " *当前" } else { "" };
                    eprintln!("{}\t{}{marker}", model.id, model.model);
                }
            }
            ChatCommand::Model(Some(id)) => {
                ensure_model_exists(&runtime.models.list_models().await?, id)?;
                model_id = id.to_owned();
                provider = runtime.models.build_model(&model_id).await?;
                eprintln!("已切换模型：{model_id}（下一回合生效）");
            }
            ChatCommand::Status => {
                let info = runtime.workspaces.get(&workspace_id)?;
                eprintln!(
                    "会话    {session_id}\n工作区  {}\n副本    {}\n模型    {model_id}\n审批    {policy:?}",
                    info.source_path, info.replica_path,
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
}
