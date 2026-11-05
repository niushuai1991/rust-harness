//! `harness chat`：终端交互式对话（docs/specs/cli-plan.md 步骤 6 实现）。
use crate::{Approval, ColorChoice};
use harness_contracts::{AppError, AppResult};
use harness_runtime::Runtime;
use std::path::PathBuf;
use std::sync::Arc;

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

pub async fn execute(_runtime: Arc<Runtime>, _args: Args) -> AppResult<()> {
    Err(AppError::new(
        "NOT_IMPLEMENTED",
        "chat 将在后续提交实现（docs/specs/cli-plan.md 步骤 6）",
    ))
}
