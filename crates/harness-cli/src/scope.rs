//! 运行作用域：messageId 回合认领、子代理作用域集合与 Lagged 降级（docs/specs/cli-plan.md）。
use harness_contracts::SessionEvent;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// 已入队，等待属于本消息的 turn.start 认领
    Pending,
    /// 已认领，等待同 messageId 的 turn.end
    Open { run_id: String },
    /// 本回合已结算
    Closed { status: String },
}

/// 作用域对一条 Durable 事件的裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 不属作用域集合，忽略
    Ignored,
    /// 作用域内普通事件，正常渲染
    Tracked,
    /// 发现新的子代理会话并已纳入作用域
    ChildAdded,
    /// 认领到本消息的回合（turn.start）
    Claimed,
    /// 本消息的回合已结算，携带 turn.end 的 status
    TurnClosed(String),
    /// 本消息在排队时被取消（inbox.cancelled）
    Cancelled,
}

pub struct RunScope {
    session_id: String,
    message_id: String,
    pub phase: Phase,
    /// 作用域集合：父会话 + 递归子代理
    pub sessions: BTreeSet<String>,
    /// 自上次 assistant.message 判定以来是否发生过 Lagged（强制补打完整正文）
    pub lagged: bool,
}

impl RunScope {
    pub fn new(session_id: &str, message_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            message_id: message_id.to_owned(),
            phase: Phase::Pending,
            sessions: BTreeSet::from([session_id.to_owned()]),
            lagged: false,
        }
    }

    pub fn in_scope(&self, session_id: &str) -> bool {
        self.sessions.contains(session_id)
    }

    /// 父会话标识。
    pub fn parent(&self) -> &str {
        &self.session_id
    }

    /// 本回合被认领的 runId（turn.start 后可用）。
    pub fn claimed_run_id(&self) -> Option<&str> {
        match &self.phase {
            Phase::Open { run_id } => Some(run_id),
            _ => None,
        }
    }

    pub fn note_lagged(&mut self) {
        self.lagged = true;
    }

    /// 处理一条 Durable 事件；子代理会话经 `subagent.created` 递归纳入作用域。
    pub fn observe(&mut self, event: &SessionEvent) -> Verdict {
        if !self.sessions.contains(&event.session_id) {
            return Verdict::Ignored;
        }
        if event.kind == "subagent.created" {
            if let Some(child) = event.data["sessionId"].as_str()
                && self.sessions.insert(child.to_owned())
            {
                return Verdict::ChildAdded;
            }
            return Verdict::Tracked;
        }
        if event.session_id == self.session_id {
            if event.kind == "inbox.cancelled"
                && event.data["id"].as_str() == Some(self.message_id.as_str())
            {
                self.phase = Phase::Closed {
                    status: "cancelled".into(),
                };
                return Verdict::Cancelled;
            }
            if event.kind == "turn.start"
                && self.phase == Phase::Pending
                && event.data["messageId"].as_str() == Some(self.message_id.as_str())
            {
                self.phase = Phase::Open {
                    run_id: event.data["runId"].as_str().unwrap_or_default().to_owned(),
                };
                return Verdict::Claimed;
            }
            if event.kind == "turn.end"
                && event.data["messageId"].as_str() == Some(self.message_id.as_str())
            {
                let status = event.data["status"].as_str().unwrap_or("error").to_owned();
                if let Phase::Open { run_id } = &self.phase
                    && event.data["runId"].as_str() == Some(run_id.as_str())
                {
                    self.phase = Phase::Closed {
                        status: status.clone(),
                    };
                    return Verdict::TurnClosed(status);
                }
            }
        }
        Verdict::Tracked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(session: &str, kind: &str, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            id: "e".into(),
            session_id: session.into(),
            seq: "1".into(),
            kind: kind.into(),
            data,
            created_at: String::new(),
        }
    }

    #[test]
    fn claims_only_own_message_and_closes_on_matching_turn_end() {
        let mut scope = RunScope::new("s1", "m1");
        // 排队回合的 turn.start（messageId 不匹配）不得认领
        let other = event(
            "s1",
            "turn.start",
            json!({"runId":"other","messageId":"m0"}),
        );
        assert_eq!(scope.observe(&other), Verdict::Tracked);
        assert_eq!(scope.phase, Phase::Pending);

        let claim = event("s1", "turn.start", json!({"runId":"r1","messageId":"m1"}));
        assert_eq!(scope.observe(&claim), Verdict::Claimed);
        assert_eq!(
            scope.phase,
            Phase::Open {
                run_id: "r1".into()
            }
        );

        // 其他会话的 turn.end 不结束本 run
        let sibling = event(
            "s2",
            "turn.end",
            json!({"runId":"x","messageId":"m9","status":"completed"}),
        );
        assert_eq!(scope.observe(&sibling), Verdict::Ignored);

        let mismatched = event(
            "s1",
            "turn.end",
            json!({"runId":"rX","messageId":"m1","status":"completed"}),
        );
        assert_eq!(scope.observe(&mismatched), Verdict::Tracked);

        let close = event(
            "s1",
            "turn.end",
            json!({"runId":"r1","messageId":"m1","status":"completed"}),
        );
        assert_eq!(
            scope.observe(&close),
            Verdict::TurnClosed("completed".into())
        );
        assert_eq!(
            scope.phase,
            Phase::Closed {
                status: "completed".into()
            }
        );
    }

    #[test]
    fn queued_cancel_sets_closed_phase() {
        let mut scope = RunScope::new("s1", "m1");
        let cancelled = event("s1", "inbox.cancelled", json!({"id":"m1"}));
        assert_eq!(scope.observe(&cancelled), Verdict::Cancelled);
        assert_eq!(
            scope.phase,
            Phase::Closed {
                status: "cancelled".into()
            }
        );
    }

    #[test]
    fn subagents_are_recursively_added_to_scope() {
        let mut scope = RunScope::new("s1", "m1");
        let spawn = event("s1", "subagent.created", json!({"sessionId":"c1"}));
        assert_eq!(scope.observe(&spawn), Verdict::ChildAdded);
        assert!(scope.in_scope("c1"));
        // 子会话再派生孙会话，同样入集
        let nested = event("c1", "subagent.created", json!({"sessionId":"g1"}));
        assert_eq!(scope.observe(&nested), Verdict::ChildAdded);
        assert!(scope.in_scope("g1"));
        assert!(!scope.in_scope("outsider"));
    }
}
