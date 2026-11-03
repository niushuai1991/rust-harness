use harness_storage::Store;
use serde_json::json;

#[tokio::test]
async fn reopen_preserves_order_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let db = Store::open(&path).await.unwrap();
    let session = db.create_session("work-a", "事务测试").await.unwrap();
    let a = db
        .append(&session.id, "user.message", json!({"content":"first"}))
        .await
        .unwrap();
    let b = db
        .append(
            &session.id,
            "assistant.message",
            json!({"content":"second"}),
        )
        .await
        .unwrap();
    assert_eq!(a.seq, "1");
    assert_eq!(b.seq, "2");
    drop(db);
    let reopened = Store::open(&path).await.unwrap();
    let events = reopened.events(&session.id, 1).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["content"], "second");
}

#[tokio::test]
async fn crash_recovery_settles_unknown_tool_once_without_reexecution() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "recover").await.unwrap();
    db.append(&session.id, "turn.start", json!({"runId":"run-a"}))
        .await
        .unwrap();
    db.append(
        &session.id,
        "tool.call",
        json!({"id":"call-a","name":"shell","arguments":{"command":"write"}}),
    )
    .await
    .unwrap();
    assert_eq!(db.recover().await.unwrap(), 1);
    assert_eq!(db.recover().await.unwrap(), 0);
    let events = db.events(&session.id, 0).await.unwrap();
    assert_eq!(events[2].kind, "tool.result");
    assert_eq!(events[2].data["status"], "unknown");
    assert_eq!(events[3].data["status"], "interrupted");
}

#[tokio::test]
async fn persisted_messages_are_searchable_with_literal_punctuation() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "untitled").await.unwrap();
    db.append(
        &session.id,
        "user.message",
        json!({"content":"repair alpha-parser"}),
    )
    .await
    .unwrap();
    assert_eq!(db.search("alpha-parser").await.unwrap()[0].id, session.id);
}

#[tokio::test]
async fn import_failure_rolls_back_the_entire_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let db = Store::open(&path).await.unwrap();
    let pool =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_bad BEFORE INSERT ON events WHEN new.kind='broken' BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&pool).await.unwrap();
    let input = concat!(
        "{\"format\":\"rust-harness.session\",\"version\":1,\"session\":{\"title\":\"import\"}}\n",
        "{\"id\":\"e1\",\"sessionId\":\"old\",\"seq\":\"1\",\"kind\":\"user.message\",\"data\":{\"content\":\"hello\"},\"createdAt\":\"date\"}\n",
        "{\"id\":\"e2\",\"sessionId\":\"old\",\"seq\":\"2\",\"kind\":\"broken\",\"data\":{},\"createdAt\":\"date\"}\n"
    );
    assert!(db.import(input, "workspace").await.is_err());
    assert!(db.sessions(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn recovery_finds_calls_committed_inside_assistant_before_tool_registration() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "crash").await.unwrap();
    db.append(&s.id, "turn.start", json!({"runId":"r"}))
        .await
        .unwrap();
    db.append(
        &s.id,
        "assistant.message",
        json!({"content":"","toolCalls":[{"id":"c","name":"write","arguments":{"path":"a"}}]}),
    )
    .await
    .unwrap();
    db.recover().await.unwrap();
    let events = db.events(&s.id, 0).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "tool.result"
        && e.data["callId"] == "c"
        && e.data["status"] == "unknown"));
}
