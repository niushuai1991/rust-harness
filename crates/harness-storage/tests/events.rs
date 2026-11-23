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

#[tokio::test]
async fn timeline_page_hides_inherited_history_behind_side_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "side").await.unwrap();
    // 继承区：父会话复制来的完整回合（seq 1-3）。
    db.append(&s.id, "turn.start", json!({"runId":"inherited"}))
        .await
        .unwrap();
    db.append(&s.id, "user.message", json!({"content":"父会话问题"}))
        .await
        .unwrap();
    db.append(&s.id, "turn.end", json!({"status":"completed"}))
        .await
        .unwrap();
    // 合成说明消息（seq 4）与封口边界（seq 5，afterSeq 记自身位置）。
    db.append(
        &s.id,
        "user.message",
        json!({"content":"以上对话继承自父会话……", "source":"side_inherited"}),
    )
    .await
    .unwrap();
    db.append(&s.id, "side.boundary", json!({"afterSeq":"5"}))
        .await
        .unwrap();
    // 侧聊自己的新回合（seq 6-9）。
    let start = db
        .append(&s.id, "turn.start", json!({"runId":"fresh"}))
        .await
        .unwrap();
    let question = db
        .append(&s.id, "user.message", json!({"content":"侧聊新问题"}))
        .await
        .unwrap();
    let answer = db
        .append(&s.id, "assistant.message", json!({"content":"侧聊回答"}))
        .await
        .unwrap();
    let end = db
        .append(&s.id, "turn.end", json!({"status":"completed"}))
        .await
        .unwrap();

    let page = db.timeline_page(&s.id, None, None).await.unwrap();
    // 只剩侧聊新回合的 4 条事件；继承历史、边界与合成消息均不出现。
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.as_str())
            .collect::<Vec<_>>(),
        ["6", "7", "8", "9"]
    );
    assert!(!page.has_older);
    assert_eq!(page.older_cursor, None);
    assert_eq!(page.through_seq, "9");
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].start_seq, start.seq);
    assert_eq!(page.turns[0].end_seq.as_deref(), Some(end.seq.as_str()));
    assert_eq!(page.turns[0].final_message_id, Some(answer.id));
    assert_eq!(page.events[1].id, question.id);
    // 全量读取不受边界影响：模型上下文、导出与事件接口仍能看到继承历史。
    let all = db.events(&s.id, 0).await.unwrap();
    assert_eq!(all.len(), 9);
}

#[tokio::test]
async fn timeline_page_returns_empty_for_side_without_new_turns() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "fresh-side").await.unwrap();
    db.append(&s.id, "user.message", json!({"content":"继承"}))
        .await
        .unwrap();
    db.append(
        &s.id,
        "user.message",
        json!({"content":"说明", "source":"side_inherited"}),
    )
    .await
    .unwrap();
    db.append(&s.id, "side.boundary", json!({"afterSeq":"3"}))
        .await
        .unwrap();
    let page = db.timeline_page(&s.id, None, None).await.unwrap();
    assert!(page.events.is_empty());
    assert!(!page.has_older);
    assert!(page.turns.is_empty());
    // through_seq 反映真实最大 seq，便于前端与全量读取对齐。
    assert_eq!(page.through_seq, "3");
}

#[tokio::test]
async fn timeline_page_identifies_completed_turn_across_page_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "cross-page").await.unwrap();
    let start = db
        .append(&s.id, "turn.start", json!({"runId":"r"}))
        .await
        .unwrap();
    db.append(&s.id, "assistant.message", json!({"content":"early"}))
        .await
        .unwrap();
    for _ in 0..85 {
        db.append(&s.id, "tool.result", json!({})).await.unwrap();
    }
    let final_message = db
        .append(&s.id, "assistant.message", json!({"content":"final"}))
        .await
        .unwrap();
    let end = db
        .append(&s.id, "turn.end", json!({"status":"completed"}))
        .await
        .unwrap();
    let page = db.timeline_page(&s.id, None, None).await.unwrap();
    assert!(page.has_older);
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].start_seq, start.seq);
    assert_eq!(page.turns[0].end_seq.as_deref(), Some(end.seq.as_str()));
    assert_eq!(
        page.turns[0].final_message_id.as_deref(),
        Some(final_message.id.as_str())
    );
}

#[tokio::test]
async fn turn_copy_reads_only_assistant_content_for_completed_turn() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "copy").await.unwrap();
    let start = db.append(&s.id, "turn.start", json!({})).await.unwrap();
    db.append(
        &s.id,
        "assistant.message",
        json!({"content":"first","reasoning":"secret"}),
    )
    .await
    .unwrap();
    db.append(&s.id, "tool.result", json!({"content":"tool"}))
        .await
        .unwrap();
    db.append(&s.id, "assistant.attempt", json!({"content":"failed"}))
        .await
        .unwrap();
    db.append(&s.id, "assistant.message", json!({"content":"last"}))
        .await
        .unwrap();
    let end = db
        .append(&s.id, "turn.end", json!({"status":"completed"}))
        .await
        .unwrap();
    let content = db
        .turn_assistant_content(&s.id, start.seq.parse().unwrap(), end.seq.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(content, vec![json!("first"), json!("last")]);
    assert!(
        db.turn_assistant_content(&s.id, 2, end.seq.parse().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn turn_copy_rejects_excessive_payload_without_partial_result() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("w", "large").await.unwrap();
    let start = db.append(&s.id, "turn.start", json!({})).await.unwrap();
    db.append(
        &s.id,
        "assistant.message",
        json!({"content":"x".repeat(8*1024*1024+1)}),
    )
    .await
    .unwrap();
    let end = db
        .append(&s.id, "turn.end", json!({"status":"completed"}))
        .await
        .unwrap();
    assert!(
        db.turn_assistant_content(&s.id, start.seq.parse().unwrap(), end.seq.parse().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn turn_navigator_directory_pages_and_summarizes_user_messages() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("nav.db")).await.unwrap();
    let session = db.create_session("work-a", "nav").await.unwrap();
    let other = db.create_session("work-a", "other").await.unwrap();
    let first = db
        .append(
            &session.id,
            "user.message",
            json!({"content":"  第一条\n 消息  "}),
        )
        .await
        .unwrap();
    db.append(
        &session.id,
        "assistant.message",
        json!({"content":"不要显示"}),
    )
    .await
    .unwrap();
    let second = db
        .append(
            &session.id,
            "user.message",
            json!({"attachments":[{"name":"file"}]}),
        )
        .await
        .unwrap();
    let third = db
        .append(
            &session.id,
            "user.message",
            json!({"content":"🧑‍💻".repeat(161)}),
        )
        .await
        .unwrap();
    db.append(&other.id, "user.message", json!({"content":"其他会话"}))
        .await
        .unwrap();
    let latest = db
        .turn_navigator_page(&session.id, None, Some(2))
        .await
        .unwrap();
    assert_eq!(
        latest
            .entries
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        vec![second.id.as_str(), third.id.as_str()]
    );
    assert_eq!(latest.entries[0].preview, "用户输入");
    assert!(latest.entries[1].preview_truncated);
    assert_eq!(latest.entries[1].preview, "🧑‍💻".repeat(160));
    assert!(latest.has_older);
    let earlier = db
        .turn_navigator_page(&session.id, Some(second.seq.parse().unwrap()), Some(2))
        .await
        .unwrap();
    assert_eq!(earlier.entries.len(), 1);
    assert_eq!(earlier.entries[0].event_id, first.id);
    assert_eq!(earlier.entries[0].preview, "第一条 消息");
    assert!(!earlier.has_older);
}

#[tokio::test]
async fn recovery_marks_unapplied_steering_failed_once_without_changing_applied() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("work-a", "steer recovery").await.unwrap();
    db.append(&session.id, "turn.start", json!({"runId":"run-a"}))
        .await
        .unwrap();
    db.append(
        &session.id,
        "steer.queued",
        json!({"id":"done","runId":"run-a","content":"applied"}),
    )
    .await
    .unwrap();
    db.append(
        &session.id,
        "steer.applied",
        json!({"id":"done","runId":"run-a"}),
    )
    .await
    .unwrap();
    db.append(
        &session.id,
        "steer.queued",
        json!({"id":"pending","runId":"run-a","content":"keep me"}),
    )
    .await
    .unwrap();
    db.recover().await.unwrap();
    db.recover().await.unwrap();
    let events = db.events(&session.id, 0).await.unwrap();
    let failures: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "steer.failed")
        .collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].data["id"], "pending");
    assert_eq!(failures[0].data["runId"], "run-a");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "steer.queued" && event.data["content"] == "keep me")
    );
}

#[tokio::test]
async fn timeline_page_carries_run_id_and_steer_status_from_outside_page() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("w", "pagination").await.unwrap();
    db.append(&session.id, "turn.start", json!({"runId":"run-1"}))
        .await
        .unwrap();
    db.append(
        &session.id,
        "steer.queued",
        json!({"id":"m","runId":"run-1","content":"补充"}),
    )
    .await
    .unwrap();
    db.append(&session.id, "assistant.message", json!({"content":"other"}))
        .await
        .unwrap();
    db.append(
        &session.id,
        "steer.applied",
        json!({"id":"m","runId":"run-1"}),
    )
    .await
    .unwrap();
    let page = db
        .timeline_page(&session.id, Some(3), Some(1))
        .await
        .unwrap();
    assert_eq!(page.events[0].data["steerStatus"], "steer.applied");
    assert_eq!(page.turns[0].run_id.as_deref(), Some("run-1"));
    assert!(page.turns[0].end_seq.is_none());
}

#[tokio::test]
async fn pending_steers_are_ordered_and_exclude_settled_or_other_runs() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let session = db.create_session("w", "steers").await.unwrap();
    for (kind, data) in [
        ("steer.queued", json!({"id":"one","runId":"r"})),
        ("steer.queued", json!({"id":"other","runId":"other"})),
        ("steer.queued", json!({"id":"done","runId":"r"})),
        ("steer.applied", json!({"id":"done","runId":"r"})),
        ("steer.queued", json!({"id":"two","runId":"r"})),
    ] {
        db.append(&session.id, kind, data).await.unwrap();
    }
    let pending = db.pending_steering(&session.id, "r").await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}
