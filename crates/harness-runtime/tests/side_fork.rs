//! 侧聊派生（/side）：复制父会话历史、标记继承边界并登记父子关系。

use harness_contracts::*;
use harness_runtime::Runtime;
use serde_json::json;

async fn setup() -> (tempfile::TempDir, std::sync::Arc<Runtime>, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "父会话")
        .await
        .unwrap();
    (temp, runtime, workspace.id, session.id)
}

#[tokio::test]
async fn side_fork_copies_history_marks_boundary_and_adopts_parent() {
    let (_temp, runtime, workspace_id, session_id) = setup().await;
    // 父会话铺三类事件：普通消息对、供 historyOnly 标记验证的子代理登记。
    runtime
        .append(
            &session_id,
            "user.message",
            json!({"content": "问一个问题"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session_id,
            "assistant.message",
            json!({"content": "答一句话"}),
        )
        .await
        .unwrap();
    runtime
        .append(
            &session_id,
            "subagent.created",
            json!({"sessionId": "legacy-child", "source": "agent"}),
        )
        .await
        .unwrap();
    runtime.set_approval_policy(&session_id, Some(ApprovalPolicy::Edit));

    let side = runtime.fork_as_side(&session_id).await.unwrap();
    assert_eq!(side.workspace_id, workspace_id);
    assert_eq!(side.title, "父会话 · 侧聊");

    // 前三条为完整复制（顺序与种类一致），subagent.created 补 historyOnly 标记。
    let events = runtime.store.events(&side.id, 0).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds[..3],
        ["user.message", "assistant.message", "subagent.created"]
    );
    assert_eq!(events[2].data["historyOnly"], json!(true));
    assert_eq!(events[2].data["sessionId"], json!("legacy-child"));

    // 第四条为合成说明消息：user.message 形态、带 source 标记。
    assert_eq!(events[3].kind, "user.message");
    assert_eq!(events[3].data["source"], json!("side_inherited"));
    assert!(events[3].data["content"].as_str().unwrap().contains("参考"));

    // 第五条为封口边界：afterSeq 记自身位置，界面时间线按 seq > afterSeq 取段。
    assert_eq!(events[4].kind, "side.boundary");
    let after_seq: i64 = events[4].data["afterSeq"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(after_seq, events[4].seq.parse::<i64>().unwrap());

    // 父子登记：侧聊挂父会话名下，父会话得到非 historyOnly 的登记事件。
    assert_eq!(
        runtime.parent_of(&side.id).as_deref(),
        Some(session_id.as_str())
    );
    assert!(runtime.children_of(&session_id).contains(&side.id));
    let parent_events = runtime.store.events(&session_id, 0).await.unwrap();
    let created = parent_events
        .iter()
        .find(|e| e.kind == "subagent.created" && e.data["sessionId"] == side.id)
        .unwrap();
    assert_eq!(created.data["source"], json!("side"));
    assert_ne!(created.data["historyOnly"], json!(true));

    // 审批策略继承。
    assert_eq!(
        runtime.approval_policy(&side.id),
        Some(ApprovalPolicy::Edit)
    );
}

#[tokio::test]
async fn side_fork_rejects_forking_from_side_session() {
    let (_temp, runtime, _workspace_id, session_id) = setup().await;
    runtime
        .append(&session_id, "user.message", json!({"content": "hi"}))
        .await
        .unwrap();
    let side = runtime.fork_as_side(&session_id).await.unwrap();
    let error = runtime.fork_as_side(&side.id).await.unwrap_err();
    assert_eq!(error.code, "AGENT_SCOPE");
}

#[tokio::test]
async fn side_fork_titles_are_numbered_for_multiple_opens() {
    let (_temp, runtime, _workspace_id, session_id) = setup().await;
    let first = runtime.fork_as_side(&session_id).await.unwrap();
    let second = runtime.fork_as_side(&session_id).await.unwrap();
    assert_eq!(first.title, "父会话 · 侧聊");
    assert_eq!(second.title, "父会话 · 侧聊 2");
}
