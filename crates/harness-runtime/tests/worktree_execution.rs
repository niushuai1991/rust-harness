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
        let mut answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}

fn call(id: &str, tool: &str, arguments: Value) -> ModelAnswer {
    ModelAnswer {
        content: String::new(),
        calls: vec![ToolCall {
            id: id.into(),
            name: tool.into(),
            arguments,
        }],
        ..Default::default()
    }
}

fn git_commit_source(source: &std::path::Path) {
    for args in [
        vec!["init", "-q"],
        vec!["add", "a.txt"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(source)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

async fn wait_pending(runtime: &Arc<Runtime>) -> ApprovalRequest {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let list = runtime.approvals.list();
            if let Some(r) = list.first() {
                return r.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn worktree_mode_edits_land_in_real_project_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    git_commit_source(&source);
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Worktree)
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
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "change".into(),
        provider.clone(),
    ));
    let pending = wait_pending(&runtime).await;
    assert_eq!(pending.tool, "write");
    runtime.approvals.resolve(&pending.id, true).unwrap();
    task.await.unwrap().unwrap();

    // 编辑直落宿主代建的会话 worktree；main 工作区保持不变。
    let worktree = runtime
        .workspaces
        .session_worktree(&workspace.id, &session.id)
        .unwrap()
        .expect("session worktree registered");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&worktree.path).join("a.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        worktree.branch,
        format!("feat/session-{}", &session.id[..8])
    );
    // 工作区差异来自 git 状态（真实落盘）。
    let changes = runtime.workspaces.diff(&workspace.id).unwrap();
    let modified = changes.iter().find(|c| c.path == "a.txt").unwrap();
    assert_eq!(modified.kind, "modified");
    assert_eq!(modified.after_text.as_deref(), Some("new"));
    // 系统提示声明执行根为 worktree 分支。
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let context = events
        .iter()
        .find(|e| e.kind == "context.snapshot")
        .map(|e| e.data["content"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(context.contains("worktree 分支"), "{context}");
    assert!(context.contains("无需自行 git worktree add"), "{context}");
}

#[tokio::test]
async fn sandbox_denied_outside_write_escalates_after_approval() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    std::fs::write(source.join("outside.txt"), "safe").unwrap();
    git_commit_source(&source);
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    let outside = source
        .join("outside.txt")
        .display()
        .to_string()
        .replace('\'', "''");
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "shell",
                json!({"command": format!("Set-Content -LiteralPath '{outside}' -Value 'escalated'")}),
            ),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "escape".into(),
        provider.clone(),
    ));
    // 第一次审批：shell 工具常规确认（沙箱内语义）。
    let first = wait_pending(&runtime).await;
    assert_eq!(first.tool, "shell");
    assert!(!first.escalated);
    runtime.approvals.resolve(&first.id, true).unwrap();
    // 第二次审批：沙箱拦截后的提权重跑。
    let second = wait_pending(&runtime).await;
    assert!(second.escalated, "escalation request expected");
    assert!(second.reason.contains("沙箱拦截"), "{}", second.reason);
    runtime.approvals.resolve(&second.id, true).unwrap();
    task.await.unwrap().unwrap();

    // 提权重跑以宿主令牌执行，写穿到执行根之外。
    assert_eq!(
        std::fs::read_to_string(source.join("outside.txt"))
            .unwrap()
            .trim_end(),
        "escalated"
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "shell")
        .map(|e| e.data.clone())
        .unwrap();
    assert_eq!(result["escalated"], json!(true));
}
