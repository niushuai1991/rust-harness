//! ask_user 提问工具：挂起等待宿主答复、无人在场自动结算与子代理路由。
use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
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
async fn setup(name: &str) -> (tempfile::TempDir, Arc<Runtime>, String) {
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
        .create_session(&workspace.id, name)
        .await
        .unwrap();
    (temp, runtime, session.id)
}
/// 等待首个 pending 提问出现并结算（宿主答复路径）。
async fn resolve_first_question(runtime: &Arc<Runtime>, answer: AskAnswer) -> AskRequest {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(request) = runtime.questions.list().first() {
                let request = request.clone();
                runtime.questions.resolve(&request.id, answer).unwrap();
                return request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn ask_user_pauses_until_resolved() {
    let (_temp, runtime, session) = setup("ask").await;
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "ask_user",
                json!({"question":"先改哪个文件？","options":[{"label":"session.rs","description":"存储层"}]}),
            ),
            ModelAnswer::default(),
        ])),
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(session.clone(), "修复".into(), model),
    );
    let question = resolve_first_question(
        &runtime,
        AskAnswer {
            text: "session.rs".into(),
            source: AskSource::Selected,
        },
    )
    .await;
    assert_eq!(question.session_id, session);
    assert_eq!(question.question, "先改哪个文件？");
    assert_eq!(question.options.len(), 1);
    task.await.unwrap().unwrap();
    assert!(runtime.questions.list().is_empty());
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "approval.request"),
        "提问免审批，不触发审批流"
    );
    let resolved = events
        .iter()
        .find(|e| e.kind == "ask.resolved")
        .expect("ask.resolved 应已落库");
    assert_eq!(resolved.data["id"], question.id);
    assert_eq!(resolved.data["source"], "selected");
    assert_eq!(resolved.data["answer"], "session.rs");
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "ask_user")
        .expect("ask_user 结果应已落库");
    assert_eq!(result.data["isError"], false);
    assert!(
        result.data["content"]
            .as_str()
            .unwrap()
            .contains("用户选择了「session.rs」")
    );
}

#[tokio::test]
async fn unattended_settles_ask_without_pending_wait() {
    let (_temp, runtime, session) = setup("unattended").await;
    runtime.set_unattended(true);
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "ask_user", json!({"question":"继续吗？"})),
            ModelAnswer::default(),
        ])),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime
            .clone()
            .run_session(session.clone(), "问".into(), model),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(runtime.questions.list().is_empty(), "无人在场不经挂起");
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "ask.request"));
    let resolved = events
        .iter()
        .find(|e| e.kind == "ask.resolved")
        .expect("无人在场也应落结算事件");
    assert_eq!(resolved.data["source"], "unattended");
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "ask_user")
        .unwrap();
    assert!(
        result.data["content"]
            .as_str()
            .unwrap()
            .contains("自行决策")
    );
}

/// 按消息内容区分父/子代理的 provider：父子共用队列会在竞争下把 ask_user 分给父会话。
struct SplitProvider {
    parent_calls: AtomicUsize,
    child_calls: AtomicUsize,
}
#[async_trait]
impl ModelProvider for SplitProvider {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut answer = if messages
            .iter()
            .any(|message| message.content == "子代理提问任务")
        {
            let n = self.child_calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                call("c-child", "ask_user", json!({"question":"子代理提问"}))
            } else {
                ModelAnswer::default()
            }
        } else {
            let n = self.parent_calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                call("c1", "agent_spawn", json!({"task":"子代理提问任务"}))
            } else {
                ModelAnswer::default()
            }
        };
        if answer.usage.is_null() {
            answer.usage = serde_json::json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}

#[tokio::test]
async fn child_agent_ask_routes_to_global_pending() {
    let (_temp, runtime, session) = setup("child-ask").await;
    let model = Arc::new(SplitProvider {
        parent_calls: AtomicUsize::new(0),
        child_calls: AtomicUsize::new(0),
    });
    let runtime_for_spawn = runtime.clone();
    tokio::spawn(async move {
        loop {
            if let Some(request) = runtime_for_spawn.approvals.list().first() {
                runtime_for_spawn
                    .approvals
                    .resolve(&request.id, true)
                    .unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(session.clone(), "派生".into(), model),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(question) = runtime
                .questions
                .list()
                .into_iter()
                .find(|q| q.session_id != session)
            {
                let child = runtime.parent_of(&question.session_id).expect("子会话归属");
                assert_eq!(child, session, "提问应来自子代理会话");
                runtime
                    .questions
                    .resolve(
                        &question.id,
                        AskAnswer {
                            text: "好的".into(),
                            source: AskSource::Text,
                        },
                    )
                    .unwrap();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.await.unwrap().unwrap();
    assert!(runtime.questions.list().is_empty());
}
