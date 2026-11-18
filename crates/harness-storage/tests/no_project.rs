use harness_storage::Store;

/// 无项目会话（workspace_id 空串哨兵）：create 允许空工作区，按空工作区过滤
/// 只返回无项目会话，全量列表包含它们；与有项目会话互不混入。
#[tokio::test]
async fn empty_workspace_sentinel_creates_and_filters_no_project_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();

    let bound = db.create_session("ws-a", "项目会话").await.unwrap();
    let no_project = db.create_session("", "无项目会话").await.unwrap();

    assert_eq!(no_project.workspace_id, "", "哨兵 workspace_id 保持空串");

    let filtered = db.sessions_filtered(Some(""), None).await.unwrap();
    assert_eq!(filtered.len(), 1, "按空工作区过滤只返回无项目会话");
    assert_eq!(filtered[0].id, no_project.id);

    let all = db.sessions(None).await.unwrap();
    assert_eq!(all.len(), 2, "全量列表包含无项目会话");

    let bound_list = db.sessions_filtered(Some("ws-a"), None).await.unwrap();
    assert_eq!(bound_list.len(), 1, "有项目过滤不混入无项目会话");
    assert_eq!(bound_list[0].id, bound.id);
}

/// 无项目会话遵循既有会话语义：标题自动生成判定、重命名、归档与恢复不受哨兵影响。
#[tokio::test]
async fn no_project_sessions_follow_common_session_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let session = db.create_session("", "新会话").await.unwrap();

    assert!(
        db.session_title_auto_pending(&session.id).await.unwrap(),
        "哨兵会话的占位标题仍可自动生成"
    );

    let renamed = db.rename_session(&session.id, "改名").await.unwrap();
    assert_eq!(renamed.title, "改名");

    db.archive_session(&session.id).await.unwrap();
    assert_eq!(
        db.sessions_filtered(Some(""), None).await.unwrap().len(),
        0,
        "归档后活跃列表不返回"
    );
    let restored = db.restore_session(&session.id).await.unwrap();
    assert_eq!(restored.archived_at, None, "恢复后回到无项目活跃列表");
}

/// move_sessions 的工作区参数仍要求非空（项目合并只作用于真实项目之间）。
#[tokio::test]
async fn move_sessions_still_requires_non_empty_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    db.create_session("", "无项目会话").await.unwrap();

    let err = db.move_sessions("", "ws-b").await.unwrap_err();
    assert_eq!(err.code, "INVALID_INPUT");
}
