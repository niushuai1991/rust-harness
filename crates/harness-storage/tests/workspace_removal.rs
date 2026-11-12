use harness_storage::Store;

/// 隐藏集合：初始为空，set 往返与去重，跨重启保留。
#[tokio::test]
async fn hidden_workspaces_round_trip_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harness.db");
    {
        let db = Store::open(&path).await.unwrap();
        assert!(db.hidden_workspaces().await.unwrap().is_empty());
        db.set_workspace_hidden("ws-a", true).await.unwrap();
        db.set_workspace_hidden("ws-b", true).await.unwrap();
        db.set_workspace_hidden("ws-a", true).await.unwrap();
        assert_eq!(
            db.hidden_workspaces().await.unwrap(),
            vec!["ws-a".to_string(), "ws-b".to_string()],
            "重复隐藏去重"
        );
    }
    let db = Store::open(&path).await.unwrap();
    assert_eq!(
        db.hidden_workspaces().await.unwrap(),
        vec!["ws-a".to_string(), "ws-b".to_string()]
    );
    db.set_workspace_hidden("ws-a", false).await.unwrap();
    assert_eq!(
        db.hidden_workspaces().await.unwrap(),
        vec!["ws-b".to_string()]
    );
}

/// 隐藏不影响会话数据：活跃与归档会话查询语义不变，归档会话仍可恢复
///（移除只改任务列表入口显示，不删记录、不归档、不迁移）。
#[tokio::test]
async fn hiding_workspace_keeps_sessions_intact() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let active = db.create_session("ws", "活跃会话").await.unwrap();
    let archived = db.create_session("ws", "归档会话").await.unwrap();
    db.archive_session(&archived.id).await.unwrap();
    db.set_workspace_hidden("ws", true).await.unwrap();

    assert_eq!(
        db.sessions_filtered(Some("ws"), None).await.unwrap()[0].id,
        active.id
    );
    assert_eq!(
        db.sessions_filtered(Some("ws"), Some(true)).await.unwrap()[0].id,
        archived.id
    );
    let restored = db.restore_session(&archived.id).await.unwrap();
    assert_eq!(restored.archived_at, None, "隐藏项目的归档会话仍可恢复");
}
