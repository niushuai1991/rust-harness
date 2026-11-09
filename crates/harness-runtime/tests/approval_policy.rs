//! 会话级审批策略（权限模式）：auto/edit/guarded/never 的裁决、事件流完整性与子代理继承。
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
async fn setup(name: &str) -> (tempfile::TempDir, Arc<Runtime>, String) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, name)
        .await
        .unwrap();
    (temp, runtime, session.id)
}
/// 等待首个 pending 审批出现并以 `false` 结算（策略未自动裁决的情形）。
async fn deny_first_pending(runtime: &Arc<Runtime>) -> ApprovalRequest {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(request) = runtime.approvals.list().first() {
                let request = request.clone();
                runtime.approvals.resolve(&request.id, false).unwrap();
                return request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn auto_policy_resolves_approval_without_pending_wait() {
    let (_temp, runtime, session) = setup("auto").await;
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Auto));
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"new"})),
            ModelAnswer::default(),
        ])),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime
            .clone()
            .run_session(session.clone(), "写".into(), model),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        runtime.approvals.list().is_empty(),
        "auto 档不经 pending 等待"
    );
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "approval.request"));
    assert!(
        events
            .iter()
            .any(|e| e.kind == "approval.resolved" && e.data["allow"] == true)
    );
}

#[tokio::test]
async fn never_policy_denies_approval_without_pending_wait() {
    let (_temp, runtime, session) = setup("never").await;
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Never));
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"new"})),
            ModelAnswer::default(),
        ])),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime
            .clone()
            .run_session(session.clone(), "写".into(), model),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(runtime.approvals.list().is_empty());
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(events.iter().any(|e| {
        e.kind == "tool.result"
            && e.data["isError"] == true
            && e.data["content"]
                .as_str()
                .unwrap()
                .contains("APPROVAL_DENIED")
    }));
    assert!(
        events
            .iter()
            .any(|e| e.kind == "approval.resolved" && e.data["allow"] == false)
    );
}

#[tokio::test]
async fn edit_policy_auto_allows_write_but_interactively_asks_shell() {
    let (_temp, runtime, session) = setup("edit").await;
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Edit));
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"new"})),
            call("c2", "shell", json!({"command":"echo ok"})),
            ModelAnswer::default(),
        ])),
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(session.clone(), "写并执行".into(), model),
    );
    let pending = deny_first_pending(&runtime).await;
    assert_eq!(pending.tool, "shell", "edit 档下 shell 仍交交互裁决");
    task.await.unwrap().unwrap();
    assert!(runtime.approvals.list().is_empty());
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "approval.resolved" && e.data["allow"] == true)
    );
}

#[tokio::test]
async fn guarded_policy_asks_once_on_dangerous_shell() {
    let (_temp, runtime, session) = setup("guarded").await;
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Guarded));
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "shell", json!({"command":"cargo test"})),
            call("c2", "shell", json!({"command":"rm -rf build dist"})),
            ModelAnswer::default(),
        ])),
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(session.clone(), "跑".into(), model),
    );
    let pending = deny_first_pending(&runtime).await;
    assert_eq!(pending.tool, "shell", "危险命令交交互裁决");
    assert_eq!(
        pending.reason,
        "自主模式检测到危险命令（remove-critical-path），需要确认本次执行。"
    );
    task.await.unwrap().unwrap();
    let events = runtime.store.events(&session, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "approval.resolved" && e.data["allow"] == true)
    );
}

#[tokio::test]
async fn agent_spawn_inherits_parent_policy() {
    let (_temp, runtime, session) = setup("inherit").await;
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Auto));
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "agent_spawn", json!({"task":"子任务"})),
            ModelAnswer::default(),
            // 不传 modelId 时子代理复用父 provider：父会话第 2 轮与子代理第 1 轮
            // 共产生 3 次 complete，两者竞争最后一个应答，CI 慢机时序下父会话
            // 可能从空队列 pop 而报 FIXTURE 错。
            ModelAnswer::default(),
        ])),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime
            .clone()
            .run_session(session.clone(), "派生".into(), model),
    )
    .await
    .unwrap()
    .unwrap();
    let child = runtime.children_of(&session).pop().expect("子会话应已登记");
    assert_eq!(runtime.approval_policy(&child), Some(ApprovalPolicy::Auto));
}

#[tokio::test]
async fn default_policy_applies_when_session_has_no_override() {
    let (_temp, runtime, session) = setup("default").await;
    runtime.set_default_approval_policy(Some(ApprovalPolicy::Auto));
    assert_eq!(
        runtime.approval_policy(&session),
        Some(ApprovalPolicy::Auto)
    );
    // 会话显式设置优先于运行时默认。
    runtime.set_approval_policy(&session, Some(ApprovalPolicy::Never));
    assert_eq!(
        runtime.approval_policy(&session),
        Some(ApprovalPolicy::Never)
    );
    runtime.set_approval_policy(&session, None);
    assert_eq!(
        runtime.approval_policy(&session),
        Some(ApprovalPolicy::Auto)
    );
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"new"})),
            ModelAnswer::default(),
        ])),
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        runtime
            .clone()
            .run_session(session.clone(), "写".into(), model),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        runtime.approvals.list().is_empty(),
        "默认档同样免 pending 等待"
    );
}
