use async_trait::async_trait;
use harness_contracts::{AppResult, ToolDefinition};
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct BlockingModel {
    started: Notify,
    release: Notify,
    requests: Mutex<Vec<Vec<ModelMessage>>>,
}

#[async_trait]
impl ModelProvider for BlockingModel {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        cancel: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let count = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(messages);
            requests.len()
        };
        if count == 1 {
            self.started.notify_one();
            tokio::select! { _ = self.release.notified() => {}, _ = cancel.cancelled() => return Err(harness_contracts::AppError::new("CANCELLED", "取消")) };
        }
        Ok(ModelAnswer {
            content: format!("回复{count}"),
            usage: serde_json::json!({"prompt_tokens":0,"completion_tokens":0}),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn steering_is_durable_and_reaches_next_request_in_same_turn() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().to_path_buf()).await.unwrap();
    let session = runtime.store.create_session("", "steering").await.unwrap();
    let model = Arc::new(BlockingModel::default());
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "初始任务".into(),
        model.clone(),
    ));
    model.started.notified().await;
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let run_id = events.iter().find(|e| e.kind == "turn.start").unwrap().data["runId"]
        .as_str()
        .unwrap();
    runtime
        .steer_session(
            &session.id,
            run_id,
            "message-1",
            "改用中文".into(),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    runtime
        .steer_session(
            &session.id,
            run_id,
            "message-1",
            "改用中文".into(),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert!(
        runtime
            .steer_session(
                &session.id,
                run_id,
                "message-1",
                "另一条".into(),
                vec![],
                vec![]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .steer_session(
                &session.id,
                "old-run",
                "message-2",
                "错误回合".into(),
                vec![],
                vec![]
            )
            .await
            .is_err()
    );
    runtime
        .steer_session(
            &session.id,
            run_id,
            "second",
            "只改桌面".into(),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    let pending = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(
        pending.iter().filter(|e| e.kind == "steer.queued").count(),
        2
    );
    assert!(!pending.iter().any(|e| e.kind == "steer.applied"));
    model.release.notify_one();
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "turn.start").count(), 1);
    assert_eq!(
        events.iter().filter(|e| e.kind == "steer.applied").count(),
        2
    );
    {
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let input: Vec<_> = requests[1]
            .iter()
            .filter(|m| m.content == "改用中文" || m.content == "只改桌面")
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(input, ["改用中文", "只改桌面"]);
    }
    assert!(
        runtime
            .steer_session(
                &session.id,
                run_id,
                "message-3",
                "太迟".into(),
                vec![],
                vec![]
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_marks_accepted_steering_unprocessed() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().to_path_buf()).await.unwrap();
    let session = runtime.store.create_session("", "cancel").await.unwrap();
    let model = Arc::new(BlockingModel::default());
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "任务".into(),
        model.clone(),
    ));
    model.started.notified().await;
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let run_id = events.iter().find(|e| e.kind == "turn.start").unwrap().data["runId"]
        .as_str()
        .unwrap();
    runtime
        .steer_session(
            &session.id,
            run_id,
            "pending",
            "补充".into(),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    runtime.cancel(&session.id);
    assert!(task.await.unwrap().is_err());
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "steer.failed" && e.data["id"] == "pending")
    );
    assert!(!events.iter().any(|e| e.kind == "steer.applied"));
}

#[tokio::test]
async fn persisted_steering_survives_cancelled_submission_future() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().to_path_buf()).await.unwrap();
    let session = runtime.store.create_session("", "orphan").await.unwrap();
    let model = Arc::new(BlockingModel::default());
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "任务".into(),
        model.clone(),
    ));
    model.started.notified().await;
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let run_id = events.iter().find(|e| e.kind == "turn.start").unwrap().data["runId"]
        .as_str()
        .unwrap();
    // 模拟受理持久化后、内存登记前请求 future 被取消。
    runtime.store.append(&session.id, "steer.queued", serde_json::json!({"id":"durable","runId":run_id,"content":"别丢失","attachments":[],"quotes":[]})).await.unwrap();
    model.release.notify_one();
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "steer.applied" && e.data["id"] == "durable")
    );
    assert!(
        model.requests.lock().unwrap()[1]
            .iter()
            .any(|m| m.content == "别丢失")
    );
}

#[derive(Default)]
struct ToolGate {
    started: Notify,
    release: Notify,
}
#[async_trait]
impl harness_runtime::ExternalTool for ToolGate {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "gate".into(),
            description: "test".into(),
            parameters: serde_json::json!({"type":"object","properties":{}}),
        }
    }
    fn requires_approval(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        _: harness_runtime::ToolContext,
        _: serde_json::Value,
        _: CancellationToken,
    ) -> AppResult<harness_contracts::ToolResult> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(harness_contracts::ToolResult {
            content: "工具完成".into(),
            is_error: false,
            diff: None,
            escalated: false,
        })
    }
}
struct ToolModel {
    requests: Mutex<Vec<Vec<ModelMessage>>>,
}
#[async_trait]
impl ModelProvider for ToolModel {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(messages);
        let calls = if requests.len() == 1 {
            vec![harness_contracts::ToolCall {
                id: "call".into(),
                name: "gate".into(),
                arguments: serde_json::json!({}),
            }]
        } else {
            vec![]
        };
        Ok(ModelAnswer {
            calls,
            usage: serde_json::json!({"prompt_tokens":0,"completion_tokens":0}),
            ..Default::default()
        })
    }
}
#[tokio::test]
async fn steering_waits_for_started_tool_and_preserves_tool_result_order() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().to_path_buf()).await.unwrap();
    let session = runtime
        .store
        .create_session("", "tool steering")
        .await
        .unwrap();
    let gate = Arc::new(ToolGate::default());
    let _registration = runtime.register_tool(gate.clone()).unwrap();
    let model = Arc::new(ToolModel {
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "任务".into(),
        model.clone(),
    ));
    gate.started.notified().await;
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let run_id = events.iter().find(|e| e.kind == "turn.start").unwrap().data["runId"]
        .as_str()
        .unwrap();
    runtime
        .steer_session(
            &session.id,
            run_id,
            "during-tool",
            "只查配置".into(),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert!(
        !runtime
            .store
            .events(&session.id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "steer.applied")
    );
    gate.release.notify_one();
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let tool = events
        .iter()
        .position(|e| e.kind == "tool.result" && e.data["callId"] == "call")
        .unwrap();
    let applied = events
        .iter()
        .position(|e| e.kind == "user.message" && e.data["steerId"] == "during-tool")
        .unwrap();
    assert!(tool < applied);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .iter()
            .any(|message| message.content == "只查配置")
    );
}
