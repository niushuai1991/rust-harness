//! CLI 审批策略（权限模式）：ask|edit|auto|never|guarded，及 TTY 缺省规则（docs/specs/cli.md）。
use clap::ValueEnum;
use harness_contracts::{AppError, AppResult, ApprovalPolicy, ApprovalRequest};

/// CLI 侧审批策略档位；单次裁决矩阵统一落在 [`ApprovalPolicy::decide`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Approval {
    /// 终端 y/n 确认（需 TTY）
    Ask,
    /// 自动放行文件编辑，其余工具仍 y/n 确认（需 TTY）
    Edit,
    /// 自动允许作用域内的审批请求
    Auto,
    /// 一律拒绝需审批工具
    Never,
    /// 自主放行，危险 shell 命令仍 y/n 确认（需 TTY）
    Guarded,
}

impl Approval {
    /// 可能弹出交互确认的档位；非 TTY 下不可用。
    fn interactive(self) -> bool {
        matches!(self, Self::Ask | Self::Edit | Self::Guarded)
    }
}

impl From<Approval> for ApprovalPolicy {
    fn from(value: Approval) -> Self {
        match value {
            Approval::Ask => Self::Ask,
            Approval::Edit => Self::Edit,
            Approval::Auto => Self::Auto,
            Approval::Never => Self::Never,
            Approval::Guarded => Self::Guarded,
        }
    }
}

/// 把预设里存的审批档位字符串解析为策略。
pub fn parse_approval(value: &str) -> AppResult<Approval> {
    let policy = ApprovalPolicy::parse(value)
        .ok_or_else(|| AppError::new("PRESET_CONFIG", format!("审批策略 {value} 不受支持")))?;
    Ok(match policy {
        ApprovalPolicy::Ask => Approval::Ask,
        ApprovalPolicy::Edit => Approval::Edit,
        ApprovalPolicy::Auto => Approval::Auto,
        ApprovalPolicy::Never => Approval::Never,
        ApprovalPolicy::Guarded => Approval::Guarded,
    })
}

/// 解析审批策略（含预设层）：`--approval` > 预设档位 > TTY 缺省规则；非 TTY 必须落到显式档位，交互档需要 TTY。
pub fn resolve_policy_with_preset(
    explicit: Option<Approval>,
    preset: Option<Approval>,
    stdin_tty: bool,
) -> AppResult<Approval> {
    resolve_policy(explicit.or(preset), stdin_tty)
}

/// 解析审批策略：`--approval` 缺省时 stdin 为 TTY 取 ask，非 TTY 必须显式指定；交互档（ask/edit/guarded）需要 TTY。
pub fn resolve_policy(explicit: Option<Approval>, stdin_tty: bool) -> AppResult<Approval> {
    match explicit {
        Some(policy) if policy.interactive() && !stdin_tty => Err(AppError::new(
            "APPROVAL_TTY",
            "--approval ask|edit|guarded 需要交互式终端（stdin 为 TTY）",
        )),
        Some(policy) => Ok(policy),
        None if stdin_tty => Ok(Approval::Ask),
        None => Err(AppError::new(
            "APPROVAL_REQUIRED",
            "stdin 非交互式，必须显式指定 --approval ask|edit|auto|never|guarded",
        )),
    }
}

/// ask 模式的确认回调：返回 true 表示允许。
pub type Confirm = dyn FnMut(&ApprovalRequest) -> AppResult<bool> + Send;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_policies_map_to_contract_decisions() {
        assert_eq!(ApprovalPolicy::from(Approval::Auto), ApprovalPolicy::Auto);
        assert_eq!(ApprovalPolicy::from(Approval::Ask), ApprovalPolicy::Ask);
        assert_eq!(ApprovalPolicy::from(Approval::Never), ApprovalPolicy::Never);
        assert_eq!(ApprovalPolicy::from(Approval::Edit), ApprovalPolicy::Edit);
        assert_eq!(
            ApprovalPolicy::from(Approval::Guarded),
            ApprovalPolicy::Guarded
        );
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
        assert_eq!(
            resolve_policy(Some(Approval::Auto), false).unwrap(),
            Approval::Auto
        );
    }

    #[test]
    fn interactive_policies_need_tty_even_when_explicit() {
        for policy in [Approval::Ask, Approval::Edit, Approval::Guarded] {
            let error = resolve_policy(Some(policy), false).unwrap_err();
            assert_eq!(error.code, "APPROVAL_TTY");
            assert_eq!(resolve_policy(Some(policy), true).unwrap(), policy);
        }
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
        assert_eq!(error.code, "APPROVAL_TTY", "预设交互档在非 TTY 同样拒绝");
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
        assert_eq!(parse_approval("edit").unwrap(), Approval::Edit);
        assert_eq!(parse_approval("Guarded").unwrap(), Approval::Guarded);
        assert_eq!(parse_approval("yolo").unwrap_err().code, "PRESET_CONFIG");
    }
}
