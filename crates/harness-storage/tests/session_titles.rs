use harness_storage::Store;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn creates_and_renames_session_titles_without_allowing_auto_title_to_overwrite_manual_name() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("titles.db")).await.unwrap();
    let session = db.create_session("work", "新会话").await.unwrap();
    assert!(db.session_title_auto_pending(&session.id).await.unwrap());

    let generated = db
        .set_generated_title(&session.id, "修复登录状态")
        .await
        .unwrap();
    assert_eq!(generated.title, "修复登录状态");
    assert!(!db.session_title_auto_pending(&session.id).await.unwrap());

    let renamed = db
        .rename_session(&session.id, "  用户指定标题  ")
        .await
        .unwrap();
    assert_eq!(renamed.title, "用户指定标题");

    let late_generation = db
        .set_generated_title(&session.id, "迟到的自动标题")
        .await
        .unwrap();
    assert_eq!(late_generation.title, "用户指定标题");
    assert!(!db.session_title_auto_pending(&session.id).await.unwrap());
}

#[tokio::test]
async fn rejects_blank_and_overlong_session_titles() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("titles.db")).await.unwrap();
    let session = db.create_session("work", "新会话").await.unwrap();

    assert_eq!(
        db.rename_session(&session.id, " \t ")
            .await
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        db.rename_session(&session.id, &"中".repeat(171))
            .await
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
}

#[tokio::test]
async fn migrates_v2_sessions_preserving_titles_and_marks_only_empty_placeholders_auto() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE sessions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE events(session_id TEXT NOT NULL,seq INTEGER NOT NULL,id TEXT NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL,created_at TEXT NOT NULL,PRIMARY KEY(session_id,seq))")
        .execute(&pool)
        .await
        .unwrap();
    for id in ["empty", "used", "named"] {
        let title = if id == "named" {
            "已命名"
        } else {
            "新会话"
        };
        sqlx::query("INSERT INTO sessions VALUES(?,?,?, 'created', 'updated')")
            .bind(id)
            .bind("work")
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO events VALUES('used',1,'event-1','user.message','{\"content\":\"你好\"}','created')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=2")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let db = Store::open(&path).await.unwrap();
    let check = SqlitePoolOptions::new()
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&check)
        .await
        .unwrap();
    assert_eq!(version, 3);
    let states: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,title,title_state FROM sessions ORDER BY id")
            .fetch_all(&check)
            .await
            .unwrap();
    assert_eq!(
        states,
        vec![
            ("empty".into(), "新会话".into(), "default".into()),
            ("named".into(), "已命名".into(), "manual".into()),
            ("used".into(), "新会话".into(), "manual".into()),
        ]
    );
    assert_eq!(db.session("empty").await.unwrap().title, "新会话");
    assert_eq!(db.sessions(None).await.unwrap().len(), 3);
    check.close().await;
}
