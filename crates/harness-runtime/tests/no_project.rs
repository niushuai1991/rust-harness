use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

struct Scripted(Mutex<VecDeque<ModelAnswer>>);
#[async_trait]
impl ModelProvider for Scripted {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut answers = self.0.lock().unwrap();
        let mut answer = answers
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}
fn call_tool(id: &str, name: &str, args: Value) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }],
        ..Default::default()
    }
}

/// 无项目会话（workspace_id 哨兵空串）回合：执行根为数据目录下会话独立的 scratch
/// root（sessions/<会话 id>/root），工具写入直接落在该目录；无工作区租约与同步语义。
#[tokio::test]
async fn scratch_session_runs_in_per_session_root() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    runtime.set_default_approval_policy(Some(ApprovalPolicy::Auto));
    let session = runtime.store.create_session("", "无项目").await.unwrap();
    let provider = Arc::new(Scripted(Mutex::new(VecDeque::from([
        call_tool("c1", "write", json!({"path":"note.md","content":"scratch"})),
        ModelAnswer {
            content: "done".into(),
            ..Default::default()
        },
    ]))));
    runtime
        .clone()
        .run_session(session.id.clone(), "写笔记".into(), provider)
        .await
        .unwrap();
    let root = temp
        .path()
        .join("host")
        .join("sessions")
        .join(&session.id)
        .join("root");
    assert_eq!(
        std::fs::read_to_string(root.join("note.md")).unwrap(),
        "scratch",
        "工具写入应落在会话独立 scratch root"
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "user.message"));
    assert!(
        events
            .iter()
            .any(|e| e.kind == "assistant.message" && e.data["content"] == "done")
    );
    // 系统上下文标注无项目语义
    assert!(events.iter().any(|e| {
        e.kind == "context.snapshot"
            && e.data["content"]
                .as_str()
                .is_some_and(|c| c.contains("无项目会话"))
    }));
}
