//! `harness sessions/diff/apply/models`：会话、差异审阅与模型管理（docs/specs/cli-plan.md 步骤 4/5 实现）。
use crate::diffview::ColorChoice;
use harness_contracts::{AppError, AppResult};
use harness_runtime::Runtime;
use std::sync::Arc;

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

pub async fn sessions(_runtime: Arc<Runtime>, _args: SessionsArgs) -> AppResult<()> {
    Err(AppError::new(
        "NOT_IMPLEMENTED",
        "sessions 将在后续提交实现（docs/specs/cli-plan.md 步骤 5）",
    ))
}

pub async fn diff(_runtime: Arc<Runtime>, _args: DiffArgs) -> AppResult<()> {
    Err(AppError::new(
        "NOT_IMPLEMENTED",
        "diff 将在后续提交实现（docs/specs/cli-plan.md 步骤 4）",
    ))
}

pub async fn apply(_runtime: Arc<Runtime>, _args: ApplyArgs) -> AppResult<()> {
    Err(AppError::new(
        "NOT_IMPLEMENTED",
        "apply 将在后续提交实现（docs/specs/cli-plan.md 步骤 4）",
    ))
}

pub async fn models(_runtime: Arc<Runtime>, _args: ModelsArgs) -> AppResult<()> {
    Err(AppError::new(
        "NOT_IMPLEMENTED",
        "models 将在后续提交实现（docs/specs/cli-plan.md 步骤 5）",
    ))
}
