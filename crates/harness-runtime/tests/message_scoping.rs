use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{MessageRole, ModelAnswer, ModelMessage, ModelProvider},
};
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
        _: harness_runtime::model::DeltaSink,
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

async fn fixture() -> (tempfile::TempDir, Arc<Runtime>, SessionInfo, Arc<Recorded>) {
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
        .create_session(&workspace.id, "scoping")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            ModelAnswer::default(),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    (temp, runtime, session, provider)
}

#[tokio::test]
async fn enqueue_returns_message_id_and_execute_carries_it_in_turn_events() {
    let (_temp, runtime, session, provider) = fixture().await;
    let message_id = runtime
        .enqueue_message(&session.id, "你好".into(), vec![], vec![])
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let queued = events
        .iter()
        .find(|e| e.kind == "inbox.queued")
        .expect("inbox.queued 已落库");
    assert_eq!(queued.data["id"].as_str(), Some(message_id.as_str()));
    assert!(!events.iter().any(|e| e.kind == "turn.start"));

    runtime
        .clone()
        .execute_message(session.id.clone(), message_id.clone(), provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let start = events
        .iter()
        .find(|e| e.kind == "turn.start")
        .expect("回合已开启");
    assert_eq!(start.data["messageId"].as_str(), Some(message_id.as_str()));
    assert!(start.data["runId"].as_str().is_some());
    let end = events
        .iter()
        .find(|e| e.kind == "turn.end")
        .expect("回合已结算");
    assert_eq!(end.data["messageId"].as_str(), Some(message_id.as_str()));
    assert_eq!(end.data["status"], "completed");
    assert!(
        events
            .iter()
            .any(|e| e.kind == "user.message" && e.data["content"] == "你好")
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancel_message_skips_queued_message_without_turn_or_model_call() {
    let (_temp, runtime, session, provider) = fixture().await;
    let message_id = runtime
        .enqueue_message(&session.id, "稍后取消".into(), vec![], vec![])
        .await
        .unwrap();
    runtime.cancel_message(&session.id, &message_id);
    let error = runtime
        .clone()
        .execute_message(session.id.clone(), message_id.clone(), provider.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert!(provider.requests.lock().unwrap().is_empty());
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "inbox.cancelled"
                && e.data["id"].as_str() == Some(message_id.as_str()))
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind == "turn.start" || e.kind == "turn.end" || e.kind == "user.message")
    );
}

#[tokio::test]
async fn cancel_message_only_affects_the_targeted_queued_message() {
    let (_temp, runtime, session, provider) = fixture().await;
    let first = runtime
        .enqueue_message(&session.id, "第一条".into(), vec![], vec![])
        .await
        .unwrap();
    let second = runtime
        .enqueue_message(&session.id, "第二条".into(), vec![], vec![])
        .await
        .unwrap();
    runtime.cancel_message(&session.id, &second);
    runtime
        .clone()
        .execute_message(session.id.clone(), first.clone(), provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let start = events
        .iter()
        .find(|e| e.kind == "turn.start")
        .expect("第一条消息正常开启回合");
    assert_eq!(start.data["messageId"].as_str(), Some(first.as_str()));
    let error = runtime
        .clone()
        .execute_message(session.id.clone(), second.clone(), provider.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

fn event(kind: &str, data: serde_json::Value) -> SessionEvent {
    SessionEvent {
        id: "e1".into(),
        session_id: "s1".into(),
        seq: "1".into(),
        kind: kind.into(),
        data,
        created_at: String::new(),
    }
}

#[test]
fn project_messages_wraps_quotes_in_xml_tags_with_index_rule() {
    // 单条引用：<quote> 不带 index，位于正文之前
    let single = [event(
        "user.message",
        serde_json::json!({"content": "请解释", "quotes": ["选中的话"]}),
    )];
    let messages = harness_runtime::agent::project_messages(&single);
    assert_eq!(messages[1].content, "<quote>\n选中的话\n</quote>\n\n请解释");

    // 多条引用：全部带 index，从 1 开始
    let multiple = [event(
        "user.message",
        serde_json::json!({"content": "请解释", "quotes": ["第一句", "第二句"]}),
    )];
    let messages = harness_runtime::agent::project_messages(&multiple);
    assert_eq!(
        messages[1].content,
        "<quote index=\"1\">\n第一句\n</quote>\n<quote index=\"2\">\n第二句\n</quote>\n\n请解释"
    );

    // 无引用（含历史事件缺字段）：正文原样
    let none = [event(
        "user.message",
        serde_json::json!({"content": "正文"}),
    )];
    let messages = harness_runtime::agent::project_messages(&none);
    assert_eq!(messages[1].content, "正文");
}

#[tokio::test]
async fn enqueue_message_carries_quotes_into_events_and_model_request() {
    let (_temp, runtime, session, provider) = fixture().await;
    let message_id = runtime
        .enqueue_message(
            &session.id,
            "解释一下".into(),
            vec![],
            vec!["第一句".into(), "第二句".into()],
        )
        .await
        .unwrap();
    let queued = runtime
        .store
        .events(&session.id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "inbox.queued")
        .expect("inbox.queued 已落库");
    assert_eq!(queued.data["quotes"][0], "第一句");
    assert_eq!(queued.data["quotes"][1], "第二句");

    runtime
        .clone()
        .execute_message(session.id.clone(), message_id, provider.clone())
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let user = events
        .iter()
        .find(|e| e.kind == "user.message")
        .expect("user.message 已落库");
    assert_eq!(user.data["quotes"][0], "第一句");
    assert_eq!(user.data["quotes"][1], "第二句");

    let requests = provider.requests.lock().unwrap();
    let sent = requests
        .iter()
        .flat_map(|m| m.iter())
        .find(|m| m.role == MessageRole::User)
        .expect("模型请求包含 user 消息");
    assert!(
        sent.content
            .contains("<quote index=\"1\">\n第一句\n</quote>")
    );
    assert!(
        sent.content
            .contains("<quote index=\"2\">\n第二句\n</quote>")
    );
    assert!(sent.content.trim_end().ends_with("解释一下"));
}
