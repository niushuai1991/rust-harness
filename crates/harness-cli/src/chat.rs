//! `harness chat`：终端交互式对话（REPL），复用 run 的单轮执行内核。
use crate::{
    approval::Approval,
    diffview::ColorChoice,
    manage::{self, ensure_model_exists},
    run,
};
use clap::ValueEnum;
use harness_contracts::{AppError, AppResult};
use harness_runtime::Runtime;
use std::{
    io::{BufRead, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 目标项目目录（默认当前目录）
    #[arg(short = 'C', long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    /// 模型 ID（默认 settings 的 cli.default_model，否则报错列出可用模型）
    #[arg(short = 'm', long)]
    pub model: Option<String>,
    /// 推理档位（必须在所选模型的支持档位内；缺省跟随模型默认）
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
    /// 颜色输出
    #[arg(long, default_value = "auto")]
    pub color: ColorChoice,
}

/// 斜杠命令解析；非斜杠输入为消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand<'a> {
    Quit,
    Help,
    Model(Option<&'a str>),
    Sessions,
    Diff,
    Apply,
    Approval(Option<&'a str>),
    Unknown(&'a str),
    Message,
}

pub fn parse_command(line: &str) -> ChatCommand<'_> {
    if !line.starts_with('/') {
        return ChatCommand::Message;
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    let rest = parts.next().map(str::trim).filter(|rest| !rest.is_empty());
    match name {
        "/quit" | "/exit" | "/q" => ChatCommand::Quit,
        "/help" | "/h" => ChatCommand::Help,
        "/model" => ChatCommand::Model(rest),
        "/sessions" => ChatCommand::Sessions,
        "/diff" => ChatCommand::Diff,
        "/apply" => ChatCommand::Apply,
        "/approval" => ChatCommand::Approval(rest),
        other => ChatCommand::Unknown(other),
    }
}

const HELP: &str = "可用命令：\n  /model [id]  查看或切换模型\n  /sessions    列出会话\n  /diff        查看副本改动\n  /apply       审阅后同步到原项目\n  /approval [auto|ask|never]  查看或切换审批策略\n  /quit        退出\n其余输入将作为任务消息发送给 Agent。";

pub async fn execute(runtime: Arc<Runtime>, args: Args) -> AppResult<()> {
    if !std::io::stdin().is_terminal() {
        return Err(AppError::new("TTY", "chat 需要交互式终端（stdin 为 TTY）"));
    }
    let mut policy = args.approval.unwrap_or(Approval::Ask);
    let (workspace_id, session_id, warning) = run::resolve_session_target(
        &runtime,
        args.cwd.as_deref(),
        args.workspace.as_deref(),
        args.session.as_deref(),
        "chat 会话",
    )
    .await?;
    let mut model_id = manage::resolve_model_id(&runtime, args.model.clone()).await?;
    let effort = args.effort.clone();
    let mut provider = runtime
        .models
        .build_model_with(&model_id, effort.clone())
        .await?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    eprintln!("chat 就绪：模型 {model_id}，会话 {session_id}。/help 查看命令，/quit 退出。");

    let stdin = std::io::stdin();
    loop {
        let mut stderr = std::io::stderr();
        write!(stderr, "你> ").ok();
        stderr.flush().ok();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_command(trimmed) {
            ChatCommand::Quit => break,
            ChatCommand::Help => eprintln!("{HELP}"),
            ChatCommand::Model(None) => {
                for model in runtime.models.list_models().await? {
                    let marker = if model.id == model_id { " *当前" } else { "" };
                    eprintln!("{}\t{}{marker}", model.id, model.model);
                }
            }
            ChatCommand::Model(Some(id)) => {
                ensure_model_exists(&runtime.models.list_models().await?, id)?;
                model_id = id.to_owned();
                provider = runtime
                    .models
                    .build_model_with(&model_id, effort.clone())
                    .await?;
                eprintln!("已切换模型：{model_id}（下一回合生效）");
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
                let context = run::TurnContext {
                    session_id: session_id.clone(),
                    workspace_id: workspace_id.clone(),
                    provider: provider.clone(),
                    policy,
                    json: false,
                    color: ColorChoice::Auto,
                };
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
    fn slash_commands_are_parsed() {
        assert_eq!(parse_command("/quit"), ChatCommand::Quit);
        assert_eq!(parse_command("/exit"), ChatCommand::Quit);
        assert_eq!(parse_command("/q"), ChatCommand::Quit);
        assert_eq!(parse_command("/help"), ChatCommand::Help);
        assert_eq!(parse_command("/model"), ChatCommand::Model(None));
        assert_eq!(parse_command("/model glm"), ChatCommand::Model(Some("glm")));
        assert_eq!(parse_command("/sessions"), ChatCommand::Sessions);
        assert_eq!(parse_command("/diff"), ChatCommand::Diff);
        assert_eq!(parse_command("/apply"), ChatCommand::Apply);
        assert_eq!(parse_command("/approval"), ChatCommand::Approval(None));
        assert_eq!(
            parse_command("/approval never"),
            ChatCommand::Approval(Some("never"))
        );
        assert_eq!(parse_command("/nope"), ChatCommand::Unknown("/nope"));
    }

    #[test]
    fn plain_text_is_a_message_and_blank_args_are_ignored() {
        assert_eq!(parse_command("修复登录页"), ChatCommand::Message);
        assert_eq!(parse_command("/model   "), ChatCommand::Model(None));
        assert_eq!(
            parse_command("/approval   never"),
            ChatCommand::Approval(Some("never"))
        );
    }
}
