use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, MessageRole, ModelAnswer, ModelMessage, ModelProvider, PartialUsageSink},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
    requests: Mutex<Vec<Vec<ModelMessage>>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.requests.lock().unwrap().push(messages);
        let mut answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = serde_json::json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}

struct PartialUsageFailure;
#[async_trait]
impl ModelProvider for PartialUsageFailure {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        Err(AppError::new("MODEL_PROTOCOL", "工具参数 JSON 无效"))
    }

    async fn complete_with_partial_usage(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
        partial_usage: PartialUsageSink,
    ) -> AppResult<ModelAnswer> {
        partial_usage(json!({"prompt_tokens":4,"completion_tokens":1}));
        Err(AppError::new("MODEL_PROTOCOL", "工具参数 JSON 无效"))
    }
}

struct CancelledWithPartialUsage {
    started: tokio::sync::Notify,
    usage: Value,
}
#[async_trait]
impl ModelProvider for CancelledWithPartialUsage {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        cancel: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.started.notify_one();
        cancel.cancelled().await;
        Err(AppError::new("CANCELLED", "用户取消"))
    }

    async fn complete_with_partial_usage(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        cancel: CancellationToken,
        _: DeltaSink,
        partial_usage: PartialUsageSink,
    ) -> AppResult<ModelAnswer> {
        self.started.notify_one();
        cancel.cancelled().await;
        partial_usage(self.usage.clone());
        Err(AppError::new("CANCELLED", "用户取消"))
    }
}
fn call(id: &str, name: &str, args: Value) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn resume_failed_turn_preserves_history_and_budget_without_a_user_message() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "resume")
        .await
        .unwrap();
    let failed = Arc::new(PartialUsageFailure);
    runtime
        .clone()
        .run_session(session.id.clone(), "完成任务".into(), failed)
        .await
        .unwrap_err();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    let failed_events = runtime.store.events(&session.id, 0).await.unwrap();
    let end_seq = failed_events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .to_string();
    runtime
        .clone()
        .resume_session(session.id.clone(), end_seq.clone(), provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "user.message").count(),
        1
    );
    assert_eq!(events.iter().filter(|e| e.kind == "turn.start").count(), 2);
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        8
    );
    assert!(
        provider.requests.lock().unwrap()[0]
            .iter()
            .any(|m| m.role == MessageRole::User && m.content.contains("完成任务"))
    );
    assert!(
        runtime
            .clone()
            .resume_session(session.id, end_seq, provider)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn manually_cancelled_turn_can_resume_when_usage_is_known() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "cancel").await.unwrap();
    let cancelled = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: json!({"prompt_tokens":3,"completion_tokens":2}),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "继续任务".into(),
        cancelled.clone(),
    ));
    cancelled.started.notified().await;
    runtime.cancel(&session.id);
    task.await.unwrap().unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events.iter().rev().find(|e| e.kind == "turn.end").unwrap();
    assert_eq!(end.data["status"], "cancelled");
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.clone(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        7
    );
}

#[tokio::test]
async fn manually_cancelled_model_without_usage_can_resume() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "cancel").await.unwrap();
    let cancelled = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: json!({}),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "任务".into(),
        cancelled.clone(),
    ));
    cancelled.started.notified().await;
    runtime.cancel(&session.id);
    task.await.unwrap().unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events.iter().rev().find(|e| e.kind == "turn.end").unwrap();
    assert_eq!(end.data["status"], "cancelled");
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.clone(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 3);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn repeated_resume_clicks_do_not_start_parallel_runs() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "retry").await.unwrap();
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "任务".into(),
            Arc::new(PartialUsageFailure),
        )
        .await
        .unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .clone();
    let provider = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: json!({"prompt_tokens":1,"completion_tokens":1}),
    });
    let first = tokio::spawn(runtime.clone().resume_session(
        session.id.clone(),
        end.clone(),
        provider.clone(),
    ));
    provider.started.notified().await;
    let error = runtime
        .clone()
        .resume_session(session.id.clone(), end, provider)
        .await
        .unwrap_err();
    assert_eq!(error.code, "RESUME_CHILD_ACTIVE");
    runtime.cancel(&session.id);
    first.await.unwrap().unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "turn.start").count(), 2);
}

#[tokio::test]
async fn resume_without_a_snapshot_does_not_reuse_a_previous_turn_budget() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "resume").await.unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"old","usedTokens":99,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"new","messageId":"new"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"new","status":"error"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.to_string(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 3);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn resume_rejects_a_newer_unrelated_budget() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "resume").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"other","usedTokens":9,"status":"active"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"error"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::new()),
        requests: Mutex::new(vec![]),
    });
    assert_eq!(
        runtime
            .clone()
            .resume_session(session.id, end.seq, provider)
            .await
            .unwrap_err()
            .code,
        "TOKEN_USAGE_UNAVAILABLE"
    );
}

#[tokio::test]
async fn repeated_failures_continue_with_the_same_budget() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "retry").await.unwrap();
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "任务".into(),
            Arc::new(PartialUsageFailure),
        )
        .await
        .unwrap_err();
    for _ in 0..2 {
        let events = runtime.store.events(&session.id, 0).await.unwrap();
        let end = events
            .iter()
            .rev()
            .find(|e| e.kind == "turn.end")
            .unwrap()
            .seq
            .to_string();
        runtime
            .clone()
            .resume_session(session.id.clone(), end, Arc::new(PartialUsageFailure))
            .await
            .unwrap_err();
    }
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "user.message").count(),
        1
    );
    assert_eq!(events.iter().filter(|e| e.kind == "turn.start").count(), 3);
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        15
    );
}

#[tokio::test]
async fn resume_legacy_failure_without_request_accounting_marks_usage_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "legacy").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":0,"status":"active"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"cancelled"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.to_string(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usageIncomplete"],
        true
    );
}

#[tokio::test]
async fn resume_old_turn_without_budget_snapshot_marks_usage_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "legacy").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"cancelled"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 3);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn resume_legacy_turn_without_message_id_preserves_history() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let session = runtime.store.create_session("", "legacy").await.unwrap();
    runtime
        .append(&session.id, "turn.start", json!({"runId":"legacy-run"}))
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"原始任务"}))
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"legacy-run","status":"interrupted"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq, provider.clone())
        .await
        .unwrap();
    assert!(
        provider.requests.lock().unwrap()[0]
            .iter()
            .any(|m| m.role == MessageRole::User && m.content == "原始任务")
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 3);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn legacy_turn_can_resume_again_after_another_failure() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "legacy").await.unwrap();
    runtime
        .append(&session.id, "turn.start", json!({"runId":"legacy"}))
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"legacy","status":"interrupted"}),
        )
        .await
        .unwrap();
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq, Arc::new(PartialUsageFailure))
        .await
        .unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .clone();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 8);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn legacy_child_with_ambiguous_budget_does_not_reset_parent_budget() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let parent = runtime.store.create_session("", "parent").await.unwrap();
    let child = runtime.store.create_session("", "child").await.unwrap();
    runtime
        .append(
            &parent.id,
            "turn.start",
            json!({"runId":"parent","messageId":"parent-message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "token.budget",
            json!({"turnId":"parent-message","usedTokens":9,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "subagent.created",
            json!({"sessionId":child.id}),
        )
        .await
        .unwrap();
    runtime
        .append(&child.id, "turn.start", json!({"runId":"child"}))
        .await
        .unwrap();
    let end = runtime
        .append(
            &child.id,
            "turn.end",
            json!({"runId":"child","status":"cancelled"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "turn.end",
            json!({"runId":"parent","status":"cancelled"}),
        )
        .await
        .unwrap();
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    let error = runtime
        .clone()
        .resume_session(child.id, end.seq, provider.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "TOKEN_USAGE_UNAVAILABLE");
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_side_conversation_with_independent_budget_can_resume() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let parent = runtime.store.create_session("", "parent").await.unwrap();
    let side = runtime.store.create_session("", "side").await.unwrap();
    runtime
        .append(
            &parent.id,
            "subagent.created",
            json!({"sessionId":side.id,"source":"side"}),
        )
        .await
        .unwrap();
    runtime
        .append(&side.id, "turn.start", json!({"runId":"side"}))
        .await
        .unwrap();
    let end = runtime
        .append(
            &side.id,
            "turn.end",
            json!({"runId":"side","status":"interrupted"}),
        )
        .await
        .unwrap();
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(side.id.clone(), end.seq, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&side.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usageIncomplete"],
        true
    );
}

#[tokio::test]
async fn resume_malformed_request_accounting_marks_usage_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "malformed").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"turnId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "model.request.accounted", json!({}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":0,"status":"active"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"error"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.clone(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usageIncomplete"],
        true
    );
}

#[tokio::test]
async fn resume_requires_raising_an_exhausted_budget() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "budget").await.unwrap();
    runtime.set_token_budget_limit(5).await.unwrap();
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "任务".into(),
            Arc::new(PartialUsageFailure),
        )
        .await
        .unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .to_string();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    assert_eq!(
        runtime
            .clone()
            .resume_session(session.id.clone(), end.clone(), provider.clone())
            .await
            .unwrap_err()
            .code,
        "TOKEN_BUDGET_EXCEEDED"
    );
    runtime.set_token_budget_limit(10).await.unwrap();
    runtime
        .clone()
        .resume_session(session.id.clone(), end, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        7
    );
}

#[tokio::test]
async fn unknown_usage_does_not_bypass_an_exhausted_known_budget() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "budget").await.unwrap();
    runtime.set_token_budget_limit(5).await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":5,"status":"exhausted"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"unknown","turnId":"message"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"cancelled"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::new()),
        requests: Mutex::new(vec![]),
    });
    assert_eq!(
        runtime
            .clone()
            .resume_session(session.id, end.seq, provider.clone())
            .await
            .unwrap_err()
            .code,
        "TOKEN_BUDGET_EXCEEDED"
    );
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resume_unknown_usage_after_restart_keeps_known_usage_and_marks_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host).await.unwrap();
    let session = runtime.store.create_session("", "restart").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"request","turnId":"message"}),
        )
        .await
        .unwrap();
    runtime.store.recover().await.unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .to_string();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end, provider.clone())
        .await
        .unwrap();
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 5);
    assert_eq!(budget.data["usageIncomplete"], true);
    assert_eq!(
        events.iter().filter(|e| e.kind == "user.message").count(),
        1
    );
}

#[tokio::test]
async fn resume_after_usage_unavailable_preserves_the_known_subtotal() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "usage").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":7,"status":"usageUnavailable"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"error"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 10);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn incomplete_usage_survives_repeated_resumes_and_known_usage_accumulates() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let session = runtime.store.create_session("", "retry").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"unknown","turnId":"message"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"cancelled"}),
        )
        .await
        .unwrap();
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq, Arc::new(PartialUsageFailure))
        .await
        .unwrap_err();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        7
    );
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .clone();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":2,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end, provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let budget = events
        .iter()
        .rev()
        .find(|e| e.kind == "token.budget")
        .unwrap();
    assert_eq!(budget.data["usedTokens"], 10);
    assert_eq!(budget.data["usageIncomplete"], true);
}

#[tokio::test]
async fn resume_after_recovery_preserves_unknown_tool_result() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let session = runtime.store.create_session("", "restart").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"request","turnId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.accounted",
            json!({"requestId":"request"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "assistant.message",
            json!({"toolCalls":[{"id":"call","name":"shell","arguments":{"command":"echo test"}}]}),
        )
        .await
        .unwrap();
    runtime.store.recover().await.unwrap();
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .to_string();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end, provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "tool.result" && e.data["status"] == "unknown")
            .count(),
        1
    );
    assert!(
        provider.requests.lock().unwrap()[0]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content.contains("未自动重试"))
    );
}

#[tokio::test]
async fn resume_settles_unfinished_tool_calls_after_runtime_error() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "tools").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"request","turnId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.accounted",
            json!({"requestId":"request"}),
        )
        .await
        .unwrap();
    runtime.append(&session.id, "assistant.message", json!({"toolCalls":[{"id":"done","name":"read","arguments":{}},{"id":"unknown","name":"shell","arguments":{}}]})).await.unwrap();
    runtime
        .append(
            &session.id,
            "tool.result",
            json!({"callId":"done","name":"read","content":"完成","isError":false}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"error"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id.clone(), end.seq.to_string(), provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "tool.result" && e.data["callId"] == "done")
            .count(),
        1
    );
    assert!(events.iter().any(|e| e.kind == "tool.result"
        && e.data["callId"] == "unknown"
        && e.data["status"] == "unknown"));
    assert!(
        provider.requests.lock().unwrap()[0]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content.contains("未自动重试"))
    );
}

#[tokio::test]
async fn resume_waits_for_a_child_that_has_not_settled() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "parent").await.unwrap();
    let child = runtime.store.create_session("", "child").await.unwrap();
    let child_provider = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: json!({"prompt_tokens":0,"completion_tokens":0}),
    });
    let child_task = tokio::spawn(runtime.clone().run_session(
        child.id.clone(),
        "子任务".into(),
        child_provider.clone(),
    ));
    child_provider.started.notified().await;
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "subagent.created",
            json!({"sessionId":child.id}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &session.id,
            "turn.end",
            json!({"runId":"run","status":"cancelled"}),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::new()),
        requests: Mutex::new(vec![]),
    });
    let error = runtime
        .clone()
        .resume_session(session.id, end.seq.to_string(), provider.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "RESUME_CHILD_ACTIVE");
    assert!(provider.requests.lock().unwrap().is_empty());
    runtime.cancel(&child.id);
    let _ = child_task.await;
}

#[tokio::test]
async fn restart_does_not_wait_forever_for_child_that_never_started() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let session = runtime.store.create_session("", "parent").await.unwrap();
    let child = runtime.store.create_session("", "child").await.unwrap();
    runtime
        .append(
            &session.id,
            "turn.start",
            json!({"runId":"run","messageId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&session.id, "user.message", json!({"content":"任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.started",
            json!({"requestId":"request","turnId":"message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "token.budget",
            json!({"turnId":"message","usedTokens":2,"status":"active"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "model.request.accounted",
            json!({"requestId":"request"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session.id,
            "subagent.created",
            json!({"sessionId":child.id}),
        )
        .await
        .unwrap();
    runtime.store.recover().await.unwrap();
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.kind == "turn.end")
        .unwrap()
        .seq
        .to_string();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(session.id, end, provider)
        .await
        .unwrap();
}

#[tokio::test]
async fn resume_child_uses_the_original_parent_budget() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let parent = runtime.store.create_session("", "parent").await.unwrap();
    let child = runtime.store.create_session("", "child").await.unwrap();
    let grandparent = runtime
        .store
        .create_session("", "grandparent")
        .await
        .unwrap();
    runtime
        .append(
            &grandparent.id,
            "subagent.created",
            json!({"sessionId":parent.id}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "turn.start",
            json!({"runId":"parent-run","messageId":"root-message"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "subagent.created",
            json!({"sessionId":child.id}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &child.id,
            "turn.start",
            json!({"runId":"child-run","messageId":"child-message","budgetTurnId":"root-message"}),
        )
        .await
        .unwrap();
    runtime
        .append(&child.id, "user.message", json!({"content":"子任务"}))
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "model.request.started",
            json!({"requestId":"request","turnId":"root-message"}),
        )
        .await
        .unwrap();
    let snapshot = json!({"turnId":"root-message","usedTokens":5,"status":"active"});
    runtime
        .append(&parent.id, "token.budget", snapshot.clone())
        .await
        .unwrap();
    runtime
        .append(&child.id, "token.budget", snapshot)
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "model.request.accounted",
            json!({"requestId":"request"}),
        )
        .await
        .unwrap();
    let end = runtime
        .append(
            &child.id,
            "turn.end",
            json!({"runId":"child-run","status":"cancelled"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &parent.id,
            "turn.end",
            json!({"runId":"parent-run","status":"completed"}),
        )
        .await
        .unwrap();
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "完成".into(),
            usage: json!({"prompt_tokens":1,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .resume_session(child.id, end.seq.to_string(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&parent.id, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|e| e.kind == "token.budget")
            .unwrap()
            .data["usedTokens"],
        7
    );
}

#[tokio::test]
async fn approved_edit_stays_in_replica_until_explicit_sync() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "read", json!({"path":"a.txt"})),
            call("c2", "write", json!({"path":"a.txt","content":"new"})),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "change".into(),
        provider.clone(),
    ));
    let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let list = runtime.approvals.list();
            if let Some(r) = list.first() {
                break r.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending.tool, "write");
    runtime.approvals.resolve(&pending.id, true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&workspace.replica_path).join("a.txt"))
            .unwrap(),
        "new"
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "tool.result").count(), 2);
    assert_eq!(events.last().unwrap().data["status"], "completed");
    assert!(
        provider.requests.lock().unwrap()[2]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content == "old")
    );
    runtime
        .workspaces
        .apply(&workspace.id, &["a.txt".into()])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn token_budget_stops_the_turn_and_skips_the_returned_tool_batch() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "budget")
        .await
        .unwrap();
    runtime.set_token_budget_limit(5).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            calls: vec![ToolCall {
                id: "todo-1".into(),
                name: "todo_write".into(),
                arguments: json!({"items":[]}),
            }],
            usage: json!({"prompt_tokens":4,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });

    let error = runtime
        .clone()
        .run_session(session.id.clone(), "循环测试".into(), provider.clone())
        .await
        .unwrap_err();

    assert_eq!(error.code, "TOKEN_BUDGET_EXCEEDED");
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(!events.iter().any(|event| event.kind == "todo.state"));
    assert!(events.iter().any(|event| {
        event.kind == "tool.result"
            && event.data["status"] == "denied"
            && event.data["isError"] == true
            && event.data["content"].as_str().unwrap().contains("未执行")
    }));
    let end = events
        .iter()
        .find(|event| event.kind == "turn.end")
        .unwrap();
    assert_eq!(end.data["error"]["code"], "TOKEN_BUDGET_EXCEEDED");
}

#[tokio::test]
async fn final_answer_is_delivered_when_its_usage_reaches_the_budget() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "final answer at budget")
        .await
        .unwrap();
    runtime.set_token_budget_limit(5).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "这是最终回答".into(),
            usage: json!({"prompt_tokens":4,"completion_tokens":1}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });

    runtime
        .clone()
        .run_session(session.id.clone(), "请回答".into(), provider.clone())
        .await
        .unwrap();

    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|event| {
        event.kind == "assistant.message" && event.data["content"] == "这是最终回答"
    }));
    let end = events
        .iter()
        .find(|event| event.kind == "turn.end")
        .unwrap();
    assert_eq!(end.data["status"], "completed");
    assert!(end.data["error"].is_null());
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn usage_is_charged_even_when_protocol_tool_decoding_fails() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "partial usage")
        .await
        .unwrap();
    runtime.set_token_budget_limit(5).await.unwrap();

    let error = runtime
        .clone()
        .run_session(
            session.id.clone(),
            "工具参数会解析失败".into(),
            Arc::new(PartialUsageFailure),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "TOKEN_BUDGET_EXCEEDED");
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|event| {
        event.kind == "token.budget"
            && event.data["usedTokens"] == 5
            && event.data["status"] == "exhausted"
    }));
}

#[tokio::test]
async fn cancellation_with_incomplete_partial_usage_keeps_cancelled_status() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "cancel during stream")
        .await
        .unwrap();
    let provider = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: Value::Null,
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "开始后取消".into(),
        provider.clone(),
    ));
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        provider.started.notified(),
    )
    .await
    .unwrap();
    runtime.cancel(&session.id);

    assert_eq!(task.await.unwrap().unwrap_err().code, "CANCELLED");
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let end = events
        .iter()
        .find(|event| event.kind == "turn.end")
        .unwrap();
    assert_eq!(end.data["status"], "cancelled");
    assert!(!events.iter().any(|event| {
        event.kind == "token.budget" && event.data["status"] == "usageUnavailable"
    }));
}

#[tokio::test]
async fn cancellation_with_complete_partial_usage_charges_before_stopping() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "cancel after usage")
        .await
        .unwrap();
    runtime.set_token_budget_limit(10).await.unwrap();
    let provider = Arc::new(CancelledWithPartialUsage {
        started: tokio::sync::Notify::new(),
        usage: json!({"prompt_tokens":3,"completion_tokens":2}),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "先取得 usage 再取消".into(),
        provider.clone(),
    ));
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        provider.started.notified(),
    )
    .await
    .unwrap();
    runtime.cancel(&session.id);

    assert_eq!(task.await.unwrap().unwrap_err().code, "CANCELLED");
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|event| {
        event.kind == "token.budget"
            && event.data["usedTokens"] == 5
            && event.data["status"] == "active"
    }));
    let end = events
        .iter()
        .find(|event| event.kind == "turn.end")
        .unwrap();
    assert_eq!(end.data["status"], "cancelled");
}

#[tokio::test]
async fn unknown_usage_stops_before_a_follow_up_model_call() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "missing usage")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "usage missing".into(),
            usage: json!({"unknown_tokens":10}),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });

    let error = runtime
        .clone()
        .run_session(session.id.clone(), "测试".into(), provider.clone())
        .await
        .unwrap_err();

    assert_eq!(error.code, "TOKEN_USAGE_UNAVAILABLE");
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn token_budget_limit_defaults_persists_and_rejects_zero() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("host");
    let runtime = Runtime::open(data_dir.clone()).await.unwrap();
    assert_eq!(runtime.token_budget_limit().await.unwrap(), 1_000_000);
    runtime.set_token_budget_limit(42).await.unwrap();
    assert_eq!(
        runtime.set_token_budget_limit(0).await.unwrap_err().code,
        "TOKEN_BUDGET_LIMIT"
    );
    drop(runtime);

    let reopened = Runtime::open(data_dir).await.unwrap();
    assert_eq!(reopened.token_budget_limit().await.unwrap(), 42);
}

#[tokio::test]
async fn denied_write_is_logged_without_mutating_the_replica() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "deny")
        .await
        .unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"denied"})),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "try write".into(),
        model,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(r) = runtime.approvals.list().first() {
                runtime.approvals.resolve(&r.id, false).unwrap();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.await.unwrap().unwrap();
    assert!(
        !std::path::Path::new(&workspace.replica_path)
            .join("a.txt")
            .exists()
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|e| {
        e.kind == "tool.result"
            && e.data["isError"] == true
            && e.data["content"]
                .as_str()
                .unwrap()
                .contains("APPROVAL_DENIED")
    }));
}

#[tokio::test]
async fn duplicate_tool_ids_are_rejected_before_settling_assistant_history() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let w = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "duplicate")
        .await
        .unwrap();
    let c = ToolCall {
        id: "same".into(),
        name: "list_files".into(),
        arguments: json!({}),
    };
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            calls: vec![c.clone(), c],
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    assert!(
        runtime
            .clone()
            .run_session(s.id.clone(), "test".into(), model)
            .await
            .is_err()
    );
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    assert!(!events.iter().any(|e| e.kind == "assistant.message"));
    assert!(events.iter().any(|e| e.kind == "assistant.attempt"));
}

struct WritesThenWaits;
#[async_trait]
impl harness_runtime::ExternalTool for WritesThenWaits {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "test_side_effect".into(),
            description: "test".into(),
            parameters: json!({"type":"object"}),
        }
    }
    fn requires_approval(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        context: harness_runtime::ToolContext,
        _: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        std::fs::write(context.root.join("effect.txt"), "already happened").unwrap();
        cancel.cancelled().await;
        Err(AppError::new("CANCELLED", "cancelled after writing"))
    }
}

#[tokio::test]
async fn cancelling_after_side_effect_records_unknown_outcome_and_releases_lease() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let w = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let s = runtime.store.create_session(&w.id, "cancel").await.unwrap();
    let _guard = runtime.register_tool(Arc::new(WritesThenWaits)).unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([call("c1", "test_side_effect", json!({}))])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(s.id.clone(), "test".into(), model),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !std::path::Path::new(&w.replica_path)
            .join("effect.txt")
            .exists()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    runtime.cancel(&s.id);
    assert!(task.await.unwrap().is_err());
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "tool.result" && e.data["status"] == "unknown")
    );
    assert!(runtime.workspaces.diff(&w.id).is_ok());
}

#[tokio::test]
async fn attachments_enter_model_history_and_spilled_output_is_readable_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let huge = "完整输出".repeat(10000);
    std::fs::write(source.join("large.txt"), &huge).unwrap();
    let data = dir.path().join("host");
    let runtime = Runtime::open(data.clone()).await.unwrap();
    let w = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "artifacts")
        .await
        .unwrap();
    let private = dir.path().join("private-host-path.txt");
    std::fs::write(&private, "附件说明").unwrap();
    let meta = runtime.artifact_import(&s.id, &private).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "attached",
                "read_artifact",
                json!({"hash":meta.hash,"offset":0,"length":32768}),
            ),
            call("large", "read", json!({"path":"large.txt"})),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session_with_attachments(
            s.id.clone(),
            "检查附件".into(),
            provider.clone(),
            vec![meta.hash.clone()],
            vec![],
        )
        .await
        .unwrap();
    {
        let requests = provider.requests.lock().unwrap();
        let history = serde_json::to_string(&requests[0]).unwrap();
        assert!(history.contains(&meta.hash));
        assert!(history.contains("private-host-path.txt"));
        assert!(!history.contains(&private.to_string_lossy().replace('\\', "\\\\")));
        assert!(
            requests[1]
                .iter()
                .any(|m| m.role == MessageRole::Tool && m.content.contains("附件说明"))
        );
    }
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    let artifact = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "large")
        .unwrap()
        .data["artifact"]
        .clone();
    assert_eq!(artifact["size"], huge.len());
    let hash = artifact["hash"].as_str().unwrap().to_owned();
    drop(runtime);
    let runtime = Runtime::open(data).await.unwrap();
    let chunk = runtime.artifact_read(&s.id, &hash, 0, 32768).await.unwrap();
    assert!(huge.starts_with(&chunk.content));
    assert!(chunk.next_offset.is_some());
    let replay_provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "spill-read",
                "read_artifact",
                json!({"hash":hash,"offset":0,"length":32768}),
            ),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            s.id.clone(),
            "继续读取完整输出".into(),
            replay_provider.clone(),
        )
        .await
        .unwrap();
    assert!(
        replay_provider.requests.lock().unwrap()[1]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content.contains("完整输出"))
    );
    let other = runtime.store.create_session(&w.id, "other").await.unwrap();
    assert!(
        runtime
            .artifact_read(&other.id, &hash, 0, 32768)
            .await
            .is_err()
    );
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer::default()])),
        requests: Mutex::new(vec![]),
    });
    assert!(
        runtime
            .clone()
            .run_session_with_attachments(
                other.id.clone(),
                "steal".into(),
                provider,
                vec![meta.hash],
                vec![]
            )
            .await
            .is_err()
    );
    assert!(
        !runtime
            .store
            .events(&other.id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "user.message")
    );
}

#[tokio::test]
async fn skills_catalog_enters_system_prompt_and_mention_injects_body() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let skill_dir = source.join(".agents").join("skills").join("review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: review\ndescription: 代码审查流程\n---\n审查正文内容",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "skills")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "请 $review 审查，注意 $PATH 是环境变量".into(),
            provider.clone(),
        )
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let loaded: Vec<_> = events.iter().filter(|e| e.kind == "skill.loaded").collect();
    assert_eq!(loaded.len(), 1, "$PATH 不在技能名单不注入");
    assert_eq!(loaded[0].data["name"], "review");
    assert_eq!(
        loaded[0].data["content"], "审查正文内容",
        "正文剥离 frontmatter"
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert!(system.content.contains("## 可用技能"));
    assert!(system.content.contains("- review: 代码审查流程"));
    assert!(system.content.contains("使用规则"));
    assert!(
        !system.content.contains("可按需读取的技能"),
        "旧裸路径清单已被 catalog 取代"
    );
    assert!(
        requests[0]
            .iter()
            .any(|m| m.role == MessageRole::User && m.content.contains("<skill name=\"review\""))
    );
}

#[tokio::test]
async fn global_agents_md_enters_system_prompt_before_project() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    std::fs::create_dir(&host).unwrap();
    std::fs::write(host.join("AGENTS.md"), "全局指令标记：回答一律使用中文。").unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("AGENTS.md"),
        "项目级指令标记：缩进统一使用空格。",
    )
    .unwrap();
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "agents-md")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(session.id.clone(), "你好".into(), provider.clone())
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert!(
        system.content.contains("全局指令标记：回答一律使用中文。"),
        "数据目录下的全局 AGENTS.md 应注入系统提示"
    );
    assert!(
        system
            .content
            .contains("项目级指令标记：缩进统一使用空格。"),
        "项目级 AGENTS.md 注入不受影响"
    );
    let global_pos = system.content.find("全局指令标记").unwrap();
    let project_pos = system.content.find("项目级指令标记").unwrap();
    assert!(global_pos < project_pos, "全局指令应先于项目级指令注入");
    assert!(
        system.content.contains(&format!(
            "{}:",
            runtime.workspaces.data_dir().join("AGENTS.md").display()
        )),
        "全局段落以数据目录下实际路径为标题，与项目级区分"
    );
}

#[tokio::test]
async fn global_agents_md_symlink_is_followed() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    std::fs::create_dir(&host).unwrap();
    let target = temp.path().join("global-instructions.md");
    std::fs::write(&target, "符号链接指令标记：回答一律使用中文。").unwrap();
    if !create_file_symlink(&target, &host.join("AGENTS.md")) {
        eprintln!("跳过：当前环境无法创建文件符号链接");
        return;
    }
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(host).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "agents-md")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(session.id.clone(), "你好".into(), provider.clone())
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert!(
        system
            .content
            .contains("符号链接指令标记：回答一律使用中文。"),
        "全局 AGENTS.md 为符号链接时应跟随读取并注入"
    );
}

/// 尝试创建文件符号链接；Windows 无开发者模式或管理员权限时返回 false，用例跳过。
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        false
    }
}

#[tokio::test]
async fn global_agents_md_truncated_at_32000_chars() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    std::fs::create_dir(&host).unwrap();
    let mut text = "甲".repeat(32_001);
    text.push_str("截断哨兵乙");
    std::fs::write(host.join("AGENTS.md"), text).unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(host).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "agents-md")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(session.id.clone(), "你好".into(), provider.clone())
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert_eq!(
        system.content.matches('甲').count(),
        32_000,
        "前 32000 字符保留"
    );
    assert!(
        !system.content.contains('乙'),
        "超出 32000 字符的内容被截断"
    );
}

#[tokio::test]
async fn global_agents_md_missing_or_empty_is_silent() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    std::fs::create_dir(&host).unwrap();
    std::fs::write(host.join("AGENTS.md"), "   \n").unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "agents-md")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(session.id.clone(), "你好".into(), provider.clone())
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert!(
        !system.content.contains(&format!(
            "{}:",
            runtime.workspaces.data_dir().join("AGENTS.md").display()
        )),
        "全局 AGENTS.md 为空白内容时不注入空段落"
    );
}

#[tokio::test]
async fn disabled_skill_is_absent_from_catalog_and_mention() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let skill_dir = source
        .join(".agents")
        .join("skills")
        .join("zskills-fixture");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\ndescription: 代码审查流程\n---\n审查正文内容",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    runtime
        .skills
        .set_enabled(".agents/skills/zskills-fixture/SKILL.md", false)
        .await
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "disabled")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer::default()])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "请 $zskills-fixture 审查".into(),
            provider.clone(),
        )
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(!events.iter().any(|e| e.kind == "skill.loaded"));
    {
        let requests = provider.requests.lock().unwrap();
        let system = &requests[0]
            .iter()
            .find(|m| m.role == MessageRole::System)
            .unwrap()
            .content;
        assert!(
            !system.contains("zskills-fixture"),
            "禁用的技能不进 catalog（用户级技能可能存在，不整段断言）"
        );
        assert!(!requests[0].iter().any(|m| m.role == MessageRole::User
            && m.content.contains("<skill name=\"zskills-fixture\"")));
    }
    let list = runtime
        .skills
        .list(std::path::Path::new(&workspace.replica_path))
        .await
        .unwrap();
    let fixture = list
        .iter()
        .find(|s| s.path == ".agents/skills/zskills-fixture/SKILL.md")
        .expect("fixture 技能在目录中");
    assert!(!fixture.enabled);
    assert!(skill_path.exists(), "禁用不动文件");
}

/// edit/write 的 tool.result 事件应携带 unified diff（桌面对话流行级差异展示的数据源）。
#[tokio::test]
async fn edit_and_write_results_carry_unified_diff() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old line\n").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "diff")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "read", json!({"path":"a.txt"})),
            call(
                "c2",
                "edit",
                json!({"path":"a.txt","oldText":"old line\n","newText":"new line\n"}),
            ),
            call("c3", "write", json!({"path":"b.txt","content":"created\n"})),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "diff".into(),
        provider.clone(),
    ));
    for _ in 0..2 {
        let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let list = runtime.approvals.list();
                if let Some(r) = list.first() {
                    break r.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        runtime.approvals.resolve(&pending.id, true).unwrap();
    }
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let edit = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "edit")
        .unwrap();
    assert_eq!(edit.data["isError"], false);
    assert_eq!(edit.data["path"], "a.txt");
    let edit_diff = edit.data["diff"].as_str().unwrap();
    assert!(edit_diff.contains("--- a.txt"));
    assert!(edit_diff.contains("-old line"));
    assert!(edit_diff.contains("+new line"));
    assert!(edit_diff.contains("@@"));
    let write = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "write")
        .unwrap();
    let write_diff = write.data["diff"].as_str().unwrap();
    assert!(write_diff.contains("--- b.txt"));
    assert!(write_diff.lines().any(|line| line.starts_with("+created")));
    assert!(
        !write_diff
            .lines()
            .any(|line| line.starts_with('-') && !line.starts_with("---")),
        "新建文件的 diff 不应有删除行"
    );
}

#[tokio::test]
async fn failed_edit_result_has_no_diff() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old line\n").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "conflict")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "read", json!({"path":"a.txt"})),
            call(
                "c2",
                "edit",
                json!({"path":"a.txt","oldText":"missing\n","newText":"x\n"}),
            ),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "conflict".into(),
        provider.clone(),
    ));
    let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let list = runtime.approvals.list();
            if let Some(r) = list.first() {
                break r.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    runtime.approvals.resolve(&pending.id, true).unwrap();
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let edit = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "edit")
        .unwrap();
    assert_eq!(edit.data["isError"], true);
    assert_eq!(edit.data["status"], "error");
    assert!(edit.data["diff"].is_null(), "失败结果不应携带 diff");
}
