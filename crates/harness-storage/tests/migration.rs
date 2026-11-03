use harness_storage::Store;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
#[tokio::test]
async fn migrates_legacy_settings_with_online_backup() {
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
    sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    let legacy = json!([{"id":"old","name":"Legacy","baseUrl":"https://example.test/v1","model":"m","credentialRef":"model/old","proxy":"http://localhost:1"}]);
    sqlx::query("INSERT INTO settings VALUES('models',?)")
        .bind(legacy.to_string())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=1")
        .execute(&pool)
        .await
        .unwrap();
    let store = Store::open(&path).await.unwrap();
    let providers = store.get("providers").await.unwrap().unwrap_or(json!([]));
    assert_eq!(providers.as_array().unwrap().len(), 1);
    assert_eq!(providers[0]["credentialRef"], "model/old");
    assert_eq!(
        providers[0]["endpoints"][0]["baseUrl"],
        "https://example.test/v1"
    );
    let models = store.get("models").await.unwrap().unwrap();
    assert_eq!(models[0]["providerId"], providers[0]["id"]);
    assert!(path.with_extension("db.v1.bak").exists());
    let backup = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(path.with_extension("db.v1.bak")),
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&backup)
        .await
        .unwrap();
    assert_eq!(version, 1);
    let saved: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='models'")
        .fetch_one(&backup)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
        legacy
    );
    backup.close().await;
    pool.close().await;
}
#[tokio::test]
async fn rejects_future_schema_without_changing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=99")
        .execute(&pool)
        .await
        .unwrap();
    let error = Store::open(&path).await.err().unwrap();
    assert_eq!(error.code, "FUTURE_SCHEMA");
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, 99);
    assert!(!path.with_extension("db.v1.bak").exists());
    pool.close().await;
}
#[tokio::test]
async fn malformed_legacy_config_rolls_back_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings VALUES('models','invalid-json')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(Store::open(&path).await.is_err());
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, 1);
    let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='models'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(raw, "invalid-json");
    assert!(path.with_extension("db.v1.bak").exists());
    pool.close().await;
}
#[tokio::test]
async fn migration_can_retry_after_repair_preserving_original_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retry.db");
    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings VALUES('models','broken')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(Store::open(&path).await.is_err());
    sqlx::query("UPDATE settings SET value='[]' WHERE key='models'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(Store::open(&path).await.is_ok());
    let backup = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(path.with_extension("db.v1.bak")),
        )
        .await
        .unwrap();
    let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='models'")
        .fetch_one(&backup)
        .await
        .unwrap();
    assert_eq!(raw, "broken");
    backup.close().await;
    pool.close().await;
}
