//! 工作副本改动的 unified diff 渲染与 apply 选择（docs/specs/cli-plan.md 步骤 4 实现）。
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}
