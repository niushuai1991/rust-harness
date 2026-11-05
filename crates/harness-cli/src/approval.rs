//! CLI 审批策略：auto|ask|never，及 TTY 缺省规则（docs/specs/cli-plan.md）。
use clap::ValueEnum;
use harness_contracts::{AppError, AppResult, ApprovalRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Approval {
    /// 自动允许作用域内的审批请求
    Auto,
    /// 终端 y/n 确认（需 TTY）
    Ask,
    /// 一律拒绝需审批工具
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

impl From<Approval> for Decision {
    fn from(value: Approval) -> Self {
        match value {
            Approval::Auto => Decision::Allow,
            Approval::Ask => Decision::Ask,
            Approval::Never => Decision::Deny,
        }
    }
}

/// 解析审批策略：`--approval` 缺省时 stdin 为 TTY 取 ask，非 TTY 必须显式指定；ask 需要 TTY。
pub fn resolve_policy(explicit: Option<Approval>, stdin_tty: bool) -> AppResult<Approval> {
    match explicit {
        Some(Approval::Ask) if !stdin_tty => Err(AppError::new(
            "APPROVAL_TTY",
            "--approval ask 需要交互式终端（stdin 为 TTY）",
        )),
        Some(policy) => Ok(policy),
        None if stdin_tty => Ok(Approval::Ask),
        None => Err(AppError::new(
            "APPROVAL_REQUIRED",
            "stdin 非交互式，必须显式指定 --approval auto|ask|never",
        )),
    }
}

/// ask 模式的确认回调：返回 true 表示允许。
pub type Confirm = dyn FnMut(&ApprovalRequest) -> AppResult<bool> + Send;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_policies_map_to_decisions() {
        assert_eq!(Decision::from(Approval::Auto), Decision::Allow);
        assert_eq!(Decision::from(Approval::Ask), Decision::Ask);
        assert_eq!(Decision::from(Approval::Never), Decision::Deny);
    }

    #[test]
    fn tty_default_is_ask_and_missing_tty_requires_explicit() {
        assert_eq!(resolve_policy(None, true).unwrap(), Approval::Ask);
        let error = resolve_policy(None, false).unwrap_err();
        assert_eq!(error.code, "APPROVAL_REQUIRED");
        assert_eq!(
            resolve_policy(Some(Approval::Never), false).unwrap(),
            Approval::Never
        );
    }

    #[test]
    fn ask_needs_tty_even_when_explicit() {
        let error = resolve_policy(Some(Approval::Ask), false).unwrap_err();
        assert_eq!(error.code, "APPROVAL_TTY");
        assert_eq!(
            resolve_policy(Some(Approval::Ask), true).unwrap(),
            Approval::Ask
        );
    }
}
