use harness_storage::Store;
use serde_json::json;

#[tokio::test]
async fn export_omits_request_snapshots_and_preserves_importable_history() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "history").await.unwrap();
    for (kind, data) in [
        ("user.message", json!({"content":"before"})),
        ("request.snapshot", json!({"messages":["secret snapshot"]})),
        ("tool.call", json!({"id":"call", "name":"read"})),
        ("tool.result", json!({"callId":"call", "content":"result"})),
        ("request.snapshot", json!({"messages":["another snapshot"]})),
        (
            "compaction.summary",
            json!({"content":"summary", "throughSeq":"5"}),
        ),
        ("assistant.message", json!({"content":"after"})),
    ] {
        db.append(&session.id, kind, data).await.unwrap();
    }
    let exported = db.export(&session.id).await.unwrap();
    assert!(!exported.contains("secret snapshot"));
    assert!(!exported.contains("another snapshot"));
    let imported = db.import(&exported, "work-b").await.unwrap();
    let events = db.events(&imported.id, 0).await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        [
            "user.message",
            "tool.call",
            "tool.result",
            "compaction.summary",
            "assistant.message"
        ]
    );
    assert_eq!(events[0].data["content"], "before");
    assert_eq!(events[2].data["content"], "result");
    assert_eq!(events[3].data["throughSeq"], "3");
    assert_eq!(events[4].data["content"], "after");
    assert_eq!(db.events(&session.id, 0).await.unwrap().len(), 7);
}

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

#[tokio::test]
async fn purge_request_snapshots_drops_only_that_kind_and_vacuum_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "compact").await.unwrap();
    for (kind, data) in [
        ("user.message", json!({"content":"hello"})),
        ("request.snapshot", json!({"messages":["big"],"tools":[1]})),
        ("assistant.message", json!({"content":"hi"})),
        (
            "request.snapshot",
            json!({"messages":["bigger"],"tools":[2]}),
        ),
        ("tool.result", json!({"callId":"c","content":"done"})),
    ] {
        db.append(&session.id, kind, data).await.unwrap();
    }
    let usage = db.request_snapshot_usage().await.unwrap();
    assert_eq!(usage.rows, 2);
    assert!(usage.bytes > 0);
    let summary = db.purge_request_snapshots().await.unwrap();
    assert_eq!(summary.rows, usage.rows);
    assert_eq!(summary.bytes, usage.bytes);
    let events = db.events(&session.id, 0).await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        ["user.message", "assistant.message", "tool.result"]
    );
    assert_eq!(events[0].data["content"], "hello");
    assert_eq!(events[2].data["content"], "done");
    // 消息全文搜索与恢复逻辑不受 purge 影响。
    assert_eq!(db.search("hello").await.unwrap()[0].id, session.id);
    assert_eq!(db.recover().await.unwrap(), 0);
    db.vacuum().await.unwrap();
    // 重复 purge 幂等。
    let again = db.purge_request_snapshots().await.unwrap();
    assert_eq!(again.rows, 0);
    assert_eq!(again.bytes, 0);
}

#[tokio::test]
async fn timeline_page_reads_latest_then_strictly_older_bounded_windows() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "long").await.unwrap();
    for index in 1..=205 {
        db.append(
            &session.id,
            "assistant.message",
            json!({"content": format!("event-{index}")}),
        )
        .await
        .unwrap();
    }

    let latest = db.timeline_page(&session.id, None, None).await.unwrap();
    assert_eq!(latest.through_seq, "205");
    assert!(latest.has_older);
    assert_eq!(latest.older_cursor.as_deref(), Some("126"));
    assert_eq!(latest.events.len(), 80);
    assert_eq!(latest.events.first().unwrap().seq, "126");
    assert_eq!(latest.events.last().unwrap().seq, "205");

    let older = db
        .timeline_page(&session.id, Some(126), Some(80))
        .await
        .unwrap();
    assert_eq!(older.through_seq, "205");
    assert!(older.has_older);
    assert_eq!(older.older_cursor.as_deref(), Some("46"));
    assert_eq!(older.events.len(), 80);
    assert_eq!(older.events.first().unwrap().seq, "46");
    assert_eq!(older.events.last().unwrap().seq, "125");
    assert!(
        latest
            .events
            .iter()
            .all(|event| older.events.iter().all(|other| other.id != event.id))
    );

    let capped = db
        .timeline_page(&session.id, None, Some(255))
        .await
        .unwrap();
    assert_eq!(capped.events.len(), 200);
    assert_eq!(capped.events.first().unwrap().seq, "6");
    assert_eq!(capped.events.last().unwrap().seq, "205");

    let empty = db.create_session("work-a", "empty").await.unwrap();
    let page = db.timeline_page(&empty.id, None, None).await.unwrap();
    assert_eq!(page.through_seq, "0");
    assert!(page.events.is_empty());
    assert!(!page.has_older);
    assert_eq!(page.older_cursor, None);
}
