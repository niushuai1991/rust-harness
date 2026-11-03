use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
    requests: Mutex<Vec<Vec<Value>>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        messages: Vec<Value>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.requests.lock().unwrap().push(messages);
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))
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
async fn approved_edit_stays_in_replica_until_explicit_sync() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source).unwrap();
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
            .any(|m| m["role"] == "tool" && m["content"] == "old")
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
async fn denied_write_is_logged_without_mutating_the_replica() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source).unwrap();
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
    let w = runtime.workspaces.create(&source).unwrap();
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
    let w = runtime.workspaces.create(&source).unwrap();
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
