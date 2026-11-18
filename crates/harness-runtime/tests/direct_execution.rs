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
async fn direct_mode_edits_land_in_source_without_replica_or_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    assert!(!std::path::Path::new(&workspace.replica_path).exists());
    let session = runtime
        .store
        .create_session_with_mode(&workspace.id, "test", Some(ExecutionMode::Direct))
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

    // 编辑直接写入源项目；不产生副本目录与 worktree。
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "new"
    );
    assert!(!source.join(".worktrees").exists());
    assert!(!std::path::Path::new(&workspace.replica_path).exists());
    // 直接模式无副本审阅语义。
    assert!(runtime.workspaces.diff(&workspace.id).unwrap().is_empty());
    // 系统提示声明执行根为源项目目录。
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let context = events
        .iter()
        .find(|e| e.kind == "context.snapshot")
        .map(|e| e.data["content"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(context.contains("源项目目录"), "{context}");
    assert!(context.contains("没有隔离副本或 worktree"), "{context}");
}

#[tokio::test]
async fn direct_mode_shell_runs_unsandboxed_after_approval() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session_with_mode(&workspace.id, "test", Some(ExecutionMode::Direct))
        .await
        .unwrap();
    // 源项目之外的位置：沙箱模式下会被拦截并触发提权审批，直接模式应一次执行成功。
    let outside = temp
        .path()
        .join("outside.txt")
        .display()
        .to_string()
        .replace('\'', "''");
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "shell",
                json!({"command": format!("Set-Content -LiteralPath '{outside}' -Value 'direct'")}),
            ),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "write outside".into(),
        provider.clone(),
    ));
    // 仅一次常规 shell 审批；无沙箱拦截后的二次提权审批。
    let first = wait_pending(&runtime).await;
    assert_eq!(first.tool, "shell");
    assert!(!first.escalated);
    runtime.approvals.resolve(&first.id, true).unwrap();
    task.await.unwrap().unwrap();
    assert!(
        runtime.approvals.list().is_empty(),
        "直接模式不应出现提权重跑审批"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("outside.txt"))
            .unwrap()
            .trim_end(),
        "direct"
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["name"] == "shell")
        .map(|e| e.data.clone())
        .unwrap();
    assert_eq!(result["isError"], json!(false));
    assert_eq!(result["escalated"], json!(false));
}

#[tokio::test]
async fn fork_inherits_execution_position_and_creates_own_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    git_commit_source(&source);
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    // 直接模式项目中的会话显式选择 worktree：仅该会话创建 worktree。
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session_with_mode(&workspace.id, "parent", Some(ExecutionMode::Worktree))
        .await
        .unwrap();
    let parent_worktree = runtime
        .workspaces
        .ensure_session_worktree(&workspace.id, &session.id)
        .unwrap();

    let fork = runtime.fork(&session.id).await.unwrap();
    assert_eq!(fork.execution_mode, Some(ExecutionMode::Worktree));
    // fork 不在命令内写工作区记录；首次执行时懒建自己的 worktree。
    assert!(
        runtime
            .workspaces
            .session_worktree(&workspace.id, &fork.id)
            .unwrap()
            .is_none()
    );
    let target = runtime
        .workspaces
        .execution_target(&workspace.id, &fork.id, fork.execution_mode, None)
        .unwrap();
    assert_ne!(target.root, std::path::PathBuf::from(&parent_worktree.path));
    let entries = std::fs::read_dir(source.join(".worktrees"))
        .unwrap()
        .count();
    assert_eq!(entries, 2);
}

#[tokio::test]
async fn child_session_reuses_parent_worktree_for_execution() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    git_commit_source(&source);
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let parent = runtime
        .store
        .create_session_with_mode(&workspace.id, "parent", Some(ExecutionMode::Worktree))
        .await
        .unwrap();
    let parent_worktree = runtime
        .workspaces
        .ensure_session_worktree(&workspace.id, &parent.id)
        .unwrap();
    let child = runtime
        .store
        .create_session_with_mode(&workspace.id, "child", Some(ExecutionMode::Worktree))
        .await
        .unwrap();
    // 子会话不写自己的登记，沿父会话解析同一 worktree 与 capability。
    let target = runtime
        .workspaces
        .execution_target(
            &workspace.id,
            &child.id,
            child.execution_mode,
            Some(&parent.id),
        )
        .unwrap();
    assert_eq!(target.root, std::path::PathBuf::from(&parent_worktree.path));
    assert_eq!(target.capability_id, parent.id);
    assert!(
        runtime
            .workspaces
            .session_worktree(&workspace.id, &child.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        std::fs::read_dir(source.join(".worktrees"))
            .unwrap()
            .count(),
        1
    );
}
