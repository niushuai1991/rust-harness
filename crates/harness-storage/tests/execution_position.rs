use harness_contracts::ExecutionMode;
use harness_storage::Store;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn session_execution_position_round_trips_and_legacy_rows_stay_null() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("harness.db")).await.unwrap();
    let direct = store
        .create_session_with_mode("ws", "直接会话", Some(ExecutionMode::Direct))
        .await
        .unwrap();
    assert_eq!(direct.execution_mode, Some(ExecutionMode::Direct));
    let loaded = store.session(&direct.id).await.unwrap();
    assert_eq!(loaded.execution_mode, Some(ExecutionMode::Direct));

    let worktree = store
        .create_session_with_mode("ws", "隔离会话", Some(ExecutionMode::Worktree))
        .await
        .unwrap();
    assert_eq!(
        store.session(&worktree.id).await.unwrap().execution_mode,
        Some(ExecutionMode::Worktree)
    );

    // 旧调用方不带执行位置：写入 NULL，运行时按工作区默认回退。
    let legacy = store.create_session("ws", "旧会话").await.unwrap();
    assert_eq!(
        store.session(&legacy.id).await.unwrap().execution_mode,
        None
    );
    let listed = store.sessions_filtered(None, None).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|s| s.id == legacy.id)
            .unwrap()
            .execution_mode,
        None
    );
}

#[tokio::test]
async fn migrates_v4_sessions_with_null_execution_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v4.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    for sql in [
        "CREATE TABLE sessions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)",
        "CREATE TABLE events(session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,seq INTEGER NOT NULL,id TEXT NOT NULL UNIQUE,kind TEXT NOT NULL,data TEXT NOT NULL,created_at TEXT NOT NULL,PRIMARY KEY(session_id,seq))",
        "CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
        "CREATE VIRTUAL TABLE message_search USING fts5(session_id UNINDEXED,content,tokenize='unicode61')",
        "ALTER TABLE sessions ADD COLUMN title_state TEXT NOT NULL DEFAULT 'manual'",
        "ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE sessions ADD COLUMN archived_at TEXT",
        "INSERT INTO sessions(id,workspace_id,title,created_at,updated_at,title_state,archived,archived_at) VALUES('legacy','ws','旧会话','2026-01-01','2026-01-01','manual',0,NULL)",
        "PRAGMA user_version = 4",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let session = store.session("legacy").await.unwrap();
    assert_eq!(session.execution_mode, None);
    // 迁移后新写入可正常使用新列。
    let created = store
        .create_session_with_mode("ws", "新会话", Some(ExecutionMode::Direct))
        .await
        .unwrap();
    assert_eq!(
        store.session(&created.id).await.unwrap().execution_mode,
        Some(ExecutionMode::Direct)
    );
}
