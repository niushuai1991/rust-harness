use harness_storage::Store;
use serde_json::json;

#[tokio::test]
async fn pinned_workspaces_starts_empty_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    assert!(db.pinned_workspaces().await.unwrap().is_empty());

    db.set_workspace_pinned("ws-a", true).await.unwrap();
    db.set_workspace_pinned("ws-b", true).await.unwrap();
    assert_eq!(db.pinned_workspaces().await.unwrap(), vec!["ws-a", "ws-b"]);

    db.set_workspace_pinned("ws-a", false).await.unwrap();
    assert_eq!(db.pinned_workspaces().await.unwrap(), vec!["ws-b"]);
}

#[tokio::test]
async fn pinned_workspaces_survives_reopen_and_dedupes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    {
        let db = Store::open(&path).await.unwrap();
        db.set_workspace_pinned("ws-a", true).await.unwrap();
        db.set_workspace_pinned("ws-a", true).await.unwrap();
    }
    let db = Store::open(&path).await.unwrap();
    assert_eq!(db.pinned_workspaces().await.unwrap(), vec!["ws-a"]);
    // 未置顶标识重复取消保持幂等，集合无重复。
    db.set_workspace_pinned("ws-missing", false).await.unwrap();
    assert_eq!(db.pinned_workspaces().await.unwrap(), vec!["ws-a"]);
}

#[tokio::test]
async fn pinned_workspaces_key_stays_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    db.set_workspace_pinned("ws-a", true).await.unwrap();
    assert_eq!(
        db.get("workspace.pinned.v1").await.unwrap(),
        Some(json!(["ws-a"]))
    );
}
