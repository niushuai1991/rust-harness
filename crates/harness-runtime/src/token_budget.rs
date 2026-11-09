use harness_contracts::{AppError, AppResult};
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tokio_util::sync::CancellationToken;

pub(crate) const DEFAULT_LIMIT_TOKENS: u64 = 1_000_000;
pub(crate) const MAX_LIMIT_TOKENS: u64 = 9_007_199_254_740_991;
pub(crate) const LIMIT_SETTING_KEY: &str = "app.token_budget_limit";

pub(crate) fn validate_limit(limit: u64) -> AppResult<u64> {
    if limit == 0 || limit > MAX_LIMIT_TOKENS {
        return Err(AppError::new(
            "TOKEN_BUDGET_LIMIT",
            format!("Token 预算必须在 1 到 {MAX_LIMIT_TOKENS} 之间"),
        ));
    }
    Ok(limit)
}

pub(crate) fn consumed_tokens(usage: &Value) -> AppResult<u64> {
    let unavailable = || {
        AppError::new(
            "TOKEN_USAGE_UNAVAILABLE",
            "模型未返回可识别的 token 用量，已停止本回合以避免绕过预算",
        )
    };
    let record = usage.as_object().ok_or_else(unavailable)?;
    let count = |value: Option<&Value>| -> AppResult<Option<u64>> {
        value
            .map(|value| value.as_u64().ok_or_else(unavailable))
            .transpose()
    };
    let nested_count = |object: Option<&Value>, key: &str| -> AppResult<Option<u64>> {
        match object {
            Some(value) => count(value.as_object().and_then(|object| object.get(key))),
            None => Ok(None),
        }
    };

    if record.contains_key("prompt_tokens") || record.contains_key("completion_tokens") {
        let input = count(record.get("prompt_tokens"))?.ok_or_else(unavailable)?;
        let output = count(record.get("completion_tokens"))?.ok_or_else(unavailable)?;
        let cached =
            nested_count(record.get("prompt_tokens_details"), "cached_tokens")?.unwrap_or_default();
        return Ok(input.saturating_sub(cached).saturating_add(output));
    }

    let input = count(record.get("input_tokens"))?.ok_or_else(unavailable)?;
    let output = count(record.get("output_tokens"))?
        .or(count(record.get("completion_tokens"))?)
        .ok_or_else(unavailable)?;
    let cache_read = count(record.get("cache_read_input_tokens"))?.unwrap_or_default();
    let cache_write = count(record.get("cache_creation_input_tokens"))?.unwrap_or_default();
    if record.contains_key("cache_read_input_tokens")
        || record.contains_key("cache_creation_input_tokens")
    {
        // Anthropic reports cache reads separately from uncached input, and cache creation is
        // billable input. Only the read/hit portion is excluded from the budget.
        return Ok(input.saturating_add(cache_write).saturating_add(output));
    }
    let cached =
        nested_count(record.get("input_tokens_details"), "cached_tokens")?.unwrap_or(cache_read);
    Ok(input.saturating_sub(cached).saturating_add(output))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetStop {
    LimitReached,
    UsageUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BudgetSnapshot {
    pub used_tokens: u64,
    pub limit_tokens: u64,
    pub stopped: Option<BudgetStop>,
}

struct BudgetState {
    used_tokens: u64,
    stopped: Option<BudgetStop>,
}

pub(crate) struct TurnBudget {
    root_session_id: String,
    turn_id: String,
    limit_tokens: u64,
    state: Mutex<BudgetState>,
    sessions: Mutex<HashMap<String, CancellationToken>>,
    updates: AsyncMutex<()>,
}

impl TurnBudget {
    pub(crate) fn new(root_session_id: String, turn_id: String, limit_tokens: u64) -> Self {
        Self {
            root_session_id,
            turn_id,
            limit_tokens,
            state: Mutex::new(BudgetState {
                used_tokens: 0,
                stopped: None,
            }),
            sessions: Mutex::new(HashMap::new()),
            updates: AsyncMutex::new(()),
        }
    }

    pub(crate) fn root_session_id(&self) -> &str {
        &self.root_session_id
    }

    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub(crate) async fn serialize_updates(&self) -> MutexGuard<'_, ()> {
        self.updates.lock().await
    }

    pub(crate) fn snapshot(&self) -> BudgetSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        BudgetSnapshot {
            used_tokens: state.used_tokens,
            limit_tokens: self.limit_tokens,
            stopped: state.stopped,
        }
    }

    pub(crate) fn add_usage(&self, tokens: u64) -> BudgetSnapshot {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.used_tokens = state.used_tokens.saturating_add(tokens);
        if state.stopped.is_none() && state.used_tokens >= self.limit_tokens {
            state.stopped = Some(BudgetStop::LimitReached);
        }
        BudgetSnapshot {
            used_tokens: state.used_tokens,
            limit_tokens: self.limit_tokens,
            stopped: state.stopped,
        }
    }

    pub(crate) fn mark_usage_unavailable(&self) -> BudgetSnapshot {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stopped.is_none() {
            state.stopped = Some(BudgetStop::UsageUnavailable);
        }
        BudgetSnapshot {
            used_tokens: state.used_tokens,
            limit_tokens: self.limit_tokens,
            stopped: state.stopped,
        }
    }

    pub(crate) fn check_available(&self) -> AppResult<()> {
        match self.snapshot().stopped {
            Some(reason) => Err(Self::error_for(reason)),
            None => Ok(()),
        }
    }

    pub(crate) fn error(&self) -> Option<AppError> {
        self.snapshot().stopped.map(Self::error_for)
    }

    pub(crate) fn register_session(&self, session_id: String, cancel: CancellationToken) {
        let stopped = self.snapshot().stopped.is_some();
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id, cancel.clone());
        if stopped {
            cancel.cancel();
        }
    }

    pub(crate) fn unregister_session(&self, session_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
    }

    pub(crate) fn cancel_others(&self, current_session: &str) {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (session, cancel) in sessions.iter() {
            if session != current_session {
                cancel.cancel();
            }
        }
    }

    fn error_for(reason: BudgetStop) -> AppError {
        match reason {
            BudgetStop::LimitReached => AppError::new(
                "TOKEN_BUDGET_EXCEEDED",
                "本轮 Token 预算已耗尽，后续模型调用已停止",
            ),
            BudgetStop::UsageUnavailable => AppError::new(
                "TOKEN_USAGE_UNAVAILABLE",
                "模型未返回可识别的 token 用量，已停止本回合以避免绕过预算",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetStop, DEFAULT_LIMIT_TOKENS, TurnBudget, consumed_tokens, validate_limit};
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn chat_completions_excludes_cached_prompt_tokens() {
        assert_eq!(
            consumed_tokens(&json!({
                "prompt_tokens": 100,
                "completion_tokens": 12,
                "prompt_tokens_details": {"cached_tokens": 60}
            }))
            .unwrap(),
            52
        );
    }

    #[test]
    fn responses_excludes_cached_input_tokens() {
        assert_eq!(
            consumed_tokens(&json!({
                "input_tokens": 100,
                "output_tokens": 12,
                "input_tokens_details": {"cached_tokens": 60}
            }))
            .unwrap(),
            52
        );
    }

    #[test]
    fn anthropic_excludes_cache_reads_and_counts_cache_writes() {
        assert_eq!(
            consumed_tokens(&json!({
                "input_tokens": 30,
                "output_tokens": 12,
                "cache_read_input_tokens": 60,
                "cache_creation_input_tokens": 10
            }))
            .unwrap(),
            52
        );
    }

    #[test]
    fn missing_or_invalid_usage_is_an_error() {
        assert!(consumed_tokens(&json!(null)).is_err());
        assert!(consumed_tokens(&json!({"input_tokens": "100", "output_tokens": 10})).is_err());
        assert!(consumed_tokens(&json!({"unknown": 100})).is_err());
    }

    #[test]
    fn budget_accumulates_all_responses_and_stops_at_the_limit() {
        let budget = TurnBudget::new("root".into(), "turn".into(), 5);
        assert_eq!(budget.add_usage(2).used_tokens, 2);
        let snapshot = budget.add_usage(3);
        assert_eq!(snapshot.used_tokens, 5);
        assert_eq!(snapshot.stopped, Some(BudgetStop::LimitReached));
        assert_eq!(
            budget.check_available().unwrap_err().code,
            "TOKEN_BUDGET_EXCEEDED"
        );
    }

    #[test]
    fn each_user_turn_gets_a_fresh_budget_and_default_is_one_million() {
        let prior_turn = TurnBudget::new("root".into(), "turn-1".into(), 100);
        prior_turn.add_usage(80);
        let next_turn = TurnBudget::new("root".into(), "turn-2".into(), 100);
        assert_eq!(next_turn.snapshot().used_tokens, 0);
        assert_eq!(DEFAULT_LIMIT_TOKENS, 1_000_000);
    }

    #[test]
    fn exhaustion_cancels_sibling_sessions_but_not_the_reporting_session() {
        let budget = TurnBudget::new("root".into(), "turn".into(), 1);
        let root_cancel = CancellationToken::new();
        let child_cancel = CancellationToken::new();
        budget.register_session("root".into(), root_cancel.clone());
        budget.register_session("child".into(), child_cancel.clone());
        budget.add_usage(1);
        budget.cancel_others("root");
        assert!(!root_cancel.is_cancelled());
        assert!(child_cancel.is_cancelled());
    }

    #[test]
    fn invalid_budget_limits_are_rejected() {
        assert!(validate_limit(0).is_err());
        assert_eq!(validate_limit(100).unwrap(), 100);
    }
}
