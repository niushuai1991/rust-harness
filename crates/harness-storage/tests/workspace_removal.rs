use harness_storage::{RemovedWorkspaceRecord, Store};

fn sample_record(workspace_id: &str, source_path: &str) -> RemovedWorkspaceRecord {
    RemovedWorkspaceRecord {
        workspace_id: workspace_id.into(),
        source_path: source_path.into(),
        name: "示例项目".into(),
        pinned: true,
        removed_at: "2026-11-13T00:00:00Z".into(),
        unarchived_session_ids: vec!["s1".into()],
        automation_ids: vec!["a1".into()],
        enabled_automation_ids: vec!["a1".into()],
    }
}

/// 批量归档工作区会话：只归档未归档的，已归档保持原状态（幂等），不影响其他工作区。
#[tokio::test]
async fn archive_workspace_sessions_archives_only_unarchived_in_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let active_a = db.create_session("ws-a", "活跃A").await.unwrap();
    let active_b = db.create_session("ws-a", "活跃B").await.unwrap();
    let other = db.create_session("ws-b", "其他项目").await.unwrap();
    let already = db.create_session("ws-a", "早已归档").await.unwrap();
    db.archive_session(&already.id).await.unwrap();

    let count = db.archive_workspace_sessions("ws-a").await.unwrap();
    assert_eq!(count, 2, "只归档未归档的两条");

    let archived = db
        .sessions_filtered(Some("ws-a"), Some(true))
        .await
        .unwrap();
    assert_eq!(archived.len(), 3);
    assert!(
        archived
            .iter()
            .all(|s| !s.archived_at.as_deref().unwrap_or("").is_empty()),
        "新归档的会话也要记录归档时刻"
    );
    assert_eq!(
        db.sessions_filtered(Some("ws-a"), None)
            .await
            .unwrap()
            .len(),
        0,
        "项目下不再有活跃会话"
    );
    // 其他工作区不受影响。
    assert_eq!(
        db.sessions_filtered(Some("ws-b"), None).await.unwrap()[0].id,
        other.id
    );

    // 幂等：再次归档返回 0。
    assert_eq!(db.archive_workspace_sessions("ws-a").await.unwrap(), 0);
    let _ = active_a;
    let _ = active_b;
}

/// 已移除项目登记：初始为空，push/pop 按 source_path 往返，跨重启保留。
#[tokio::test]
async fn removed_workspace_records_round_trip_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harness.db");
    let record = sample_record("ws-old", r"D:\code\demo");
    {
        let db = Store::open(&path).await.unwrap();
        assert!(db.removed_workspaces().await.unwrap().is_empty());
        db.push_removed_workspace(&record).await.unwrap();
        db.push_removed_workspace(&sample_record("ws-old2", r"D:\code\other"))
            .await
            .unwrap();
    }
    let db = Store::open(&path).await.unwrap();
    assert_eq!(db.removed_workspaces().await.unwrap().len(), 2);

    let popped = db
        .pop_removed_workspace(r"D:\code\demo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(popped.workspace_id, "ws-old");
    assert_eq!(popped.name, "示例项目");
    assert!(popped.pinned);
    assert_eq!(popped.unarchived_session_ids, vec!["s1".to_string()]);
    assert_eq!(db.removed_workspaces().await.unwrap().len(), 1);
    assert!(
        db.pop_removed_workspace(r"D:\code\demo")
            .await
            .unwrap()
            .is_none()
    );
}

/// 会话迁移：全部会话换绑新工作区，restore_ids 中的会话恢复未归档，其余保持归档。
#[tokio::test]
async fn reassign_sessions_moves_workspace_and_restores_selected_archives() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let was_active = db.create_session("ws-old", "迁移时活跃").await.unwrap();
    let was_archived = db.create_session("ws-old", "迁移前已归档").await.unwrap();
    db.archive_session(&was_archived.id).await.unwrap();
    db.archive_workspace_sessions("ws-old").await.unwrap();

    let moved = db
        .reassign_sessions("ws-old", "ws-new", std::slice::from_ref(&was_active.id))
        .await
        .unwrap();
    assert_eq!(moved, 2);

    let active = db.sessions_filtered(Some("ws-new"), None).await.unwrap();
    assert_eq!(
        active.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec![was_active.id.as_str()],
        "restore_ids 中的会话恢复为活跃"
    );
    let archived = db
        .sessions_filtered(Some("ws-new"), Some(true))
        .await
        .unwrap();
    assert_eq!(
        archived.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec![was_archived.id.as_str()],
        "此前已归档的会话保持归档"
    );
    assert!(
        db.sessions_filtered(Some("ws-old"), Some(false))
            .await
            .unwrap()
            .is_empty()
    );
}
