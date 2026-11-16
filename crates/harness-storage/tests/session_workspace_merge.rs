use harness_storage::Store;

/// 会话迁移：活跃与归档会话一并挂到目标工作区，返回迁移数；源工作区清空。
#[tokio::test]
async fn move_sessions_moves_active_and_archived_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let active = db.create_session("ws-from", "活跃会话").await.unwrap();
    let archived = db.create_session("ws-from", "归档会话").await.unwrap();
    db.archive_session(&archived.id).await.unwrap();
    db.create_session("ws-to", "保留方会话").await.unwrap();

    let moved = db.move_sessions("ws-from", "ws-to").await.unwrap();
    assert_eq!(moved, 2, "活跃与归档会话都应迁移");

    let all = db
        .sessions_filtered(Some("ws-to"), Some(false))
        .await
        .unwrap();
    assert_eq!(all.len(), 3, "目标工作区应包含原会话与迁移会话");
    assert!(
        all.iter().any(|s| s.id == active.id),
        "活跃会话已挂到目标工作区"
    );
    assert!(
        all.iter().any(|s| s.id == archived.id),
        "归档会话保持归档状态迁移"
    );
    assert!(
        db.sessions_filtered(Some("ws-from"), Some(false))
            .await
            .unwrap()
            .is_empty(),
        "源工作区不再持有会话"
    );
}

/// 源工作区无会话时迁移为幂等空操作（合并重试场景），返回 0 且不报错。
#[tokio::test]
async fn move_sessions_is_idempotent_without_from_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    db.create_session("ws-to", "保留方会话").await.unwrap();

    assert_eq!(db.move_sessions("ws-from", "ws-to").await.unwrap(), 0);
    assert_eq!(db.move_sessions("ws-from", "ws-to").await.unwrap(), 0);
    assert_eq!(
        db.sessions_filtered(Some("ws-to"), Some(false))
            .await
            .unwrap()
            .len(),
        1
    );
}

/// 迁移结果跨数据库重开持久。
#[tokio::test]
async fn move_sessions_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harness.db");
    {
        let db = Store::open(&path).await.unwrap();
        db.create_session("ws-from", "会话").await.unwrap();
        db.move_sessions("ws-from", "ws-to").await.unwrap();
    }
    let db = Store::open(&path).await.unwrap();
    let sessions = db
        .sessions_filtered(Some("ws-to"), Some(false))
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workspace_id, "ws-to");
}
