use harness_storage::Store;
use sqlx::sqlite::SqlitePoolOptions;

/// 归档会话：默认列表隐藏、归档列表可见，archived_at 记录归档时刻且不改变 updated_at。
#[tokio::test]
async fn archive_hides_session_from_default_list_and_records_archived_at() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let keep = db.create_session("ws", "保留会话").await.unwrap();
    let gone = db.create_session("ws", "归档会话").await.unwrap();

    let archived = db.archive_session(&gone.id).await.unwrap();
    assert!(
        !archived.archived_at.as_deref().unwrap_or("").is_empty(),
        "归档应记录归档时刻"
    );
    assert_eq!(
        archived.updated_at, gone.updated_at,
        "归档不改变 updated_at"
    );

    let active = db.sessions(None, None).await.unwrap();
    assert_eq!(
        active.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec![keep.id.as_str()]
    );
    let archived_list = db.sessions(None, Some(true)).await.unwrap();
    assert_eq!(archived_list.len(), 1);
    assert_eq!(archived_list[0].id, gone.id);
    // 显式请求 Some(false) 表示不过滤（返回全部，含活跃与归档）。
    assert_eq!(db.sessions(None, Some(false)).await.unwrap().len(), 2);
    // 存档不是删除：会话条目本身仍可读。
    assert_eq!(db.session(&gone.id).await.unwrap().id, gone.id);
}

/// 恢复会话：回到默认列表并清空归档时刻。
#[tokio::test]
async fn restore_returns_session_to_default_list_and_clears_archived_at() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let session = db.create_session("ws", "归档会话").await.unwrap();
    db.archive_session(&session.id).await.unwrap();

    let restored = db.restore_session(&session.id).await.unwrap();
    assert_eq!(restored.archived_at, None);
    assert_eq!(db.sessions(None, None).await.unwrap().len(), 1);
    assert!(db.sessions(None, Some(true)).await.unwrap().is_empty());
}

/// 归档状态跨重启保留。
#[tokio::test]
async fn archive_state_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("harness.db");
    let id = {
        let db = Store::open(&path).await.unwrap();
        let session = db.create_session("ws", "归档会话").await.unwrap();
        db.archive_session(&session.id).await.unwrap();
        session.id
    };
    let db = Store::open(&path).await.unwrap();
    assert!(db.sessions(None, None).await.unwrap().is_empty());
    let archived = db.sessions(None, Some(true)).await.unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, id);
    assert!(!archived[0].archived_at.as_deref().unwrap_or("").is_empty());
}

/// 不存在的会话归档/恢复一律 NOT_FOUND。
#[tokio::test]
async fn archiving_missing_session_reports_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    assert_eq!(
        db.archive_session("missing").await.unwrap_err().code,
        "NOT_FOUND"
    );
    assert_eq!(
        db.restore_session("missing").await.unwrap_err().code,
        "NOT_FOUND"
    );
}

/// 重复归档幂等：仍只有一个归档条目，archived_at 保持有值。
#[tokio::test]
async fn archive_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let session = db.create_session("ws", "归档会话").await.unwrap();
    db.archive_session(&session.id).await.unwrap();
    let again = db.archive_session(&session.id).await.unwrap();
    assert!(!again.archived_at.as_deref().unwrap_or("").is_empty());
    assert_eq!(db.sessions(None, Some(true)).await.unwrap().len(), 1);
}

/// v3 → v4：为 sessions 表补 archived/archived_at 列，既有会话默认未归档。
#[tokio::test]
async fn migrates_v3_sessions_with_archive_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE sessions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,title_state TEXT NOT NULL DEFAULT 'manual')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE events(session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,seq INTEGER NOT NULL,id TEXT NOT NULL UNIQUE,kind TEXT NOT NULL,data TEXT NOT NULL,created_at TEXT NOT NULL,PRIMARY KEY(session_id,seq))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE VIRTUAL TABLE message_search USING fts5(session_id UNINDEXED,content,tokenize='unicode61')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions(id,workspace_id,title,created_at,updated_at,title_state) VALUES('s1','ws','旧会话','t1','t2','manual')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=3")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let db = Store::open(&path).await.unwrap();
    let active = db.sessions(None, None).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "s1");
    assert_eq!(active[0].archived_at, None, "旧会话默认未归档");
    assert!(db.sessions(None, Some(true)).await.unwrap().is_empty());

    let check = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .read_only(true),
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&check)
        .await
        .unwrap();
    assert_eq!(version, 4);
    check.close().await;
}
