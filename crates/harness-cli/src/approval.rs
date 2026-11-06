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

/// 解析审批策略（含预设层）：`--approval` > 预设档位 > TTY 缺省规则；非 TTY 必须落到显式档位，ask 需要 TTY。
pub fn resolve_policy_with_preset(
    explicit: Option<Approval>,
    preset: Option<Approval>,
    stdin_tty: bool,
) -> AppResult<Approval> {
    resolve_policy(explicit.or(preset), stdin_tty)
}

/// 把预设里存的审批档位字符串解析为策略。
pub fn parse_approval(value: &str) -> AppResult<Approval> {
    Approval::from_str(value, true)
        .map_err(|_| AppError::new("PRESET_CONFIG", format!("审批策略 {value} 不受支持")))
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

    #[test]
    fn explicit_flag_overshadows_preset_layer() {
        assert_eq!(
            resolve_policy_with_preset(Some(Approval::Never), Some(Approval::Auto), true).unwrap(),
            Approval::Never,
            "显式 --approval 恒优先于预设档位"
        );
    }

    #[test]
    fn preset_fills_before_tty_rules() {
        assert_eq!(
            resolve_policy_with_preset(None, Some(Approval::Auto), false).unwrap(),
            Approval::Auto,
            "非 TTY 时预设 auto 免显式指定"
        );
        let error = resolve_policy_with_preset(None, Some(Approval::Ask), false).unwrap_err();
        assert_eq!(error.code, "APPROVAL_TTY", "预设 ask 在非 TTY 同样拒绝");
        assert_eq!(
            resolve_policy_with_preset(None, None, true).unwrap(),
            Approval::Ask,
            "无预设回落 TTY 规则"
        );
        let error = resolve_policy_with_preset(None, None, false).unwrap_err();
        assert_eq!(error.code, "APPROVAL_REQUIRED");
    }

    #[test]
    fn parse_approval_accepts_known_set_only() {
        assert_eq!(parse_approval("auto").unwrap(), Approval::Auto);
        assert_eq!(parse_approval("ASK").unwrap(), Approval::Ask);
        assert_eq!(parse_approval("never").unwrap(), Approval::Never);
        assert_eq!(parse_approval("yolo").unwrap_err().code, "PRESET_CONFIG");
    }
}
