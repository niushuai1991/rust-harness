use harness_storage::Store;
use serde_json::json;

#[tokio::test]
async fn latest_states_returns_only_newest_entry_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let s = db.create_session("work-a", "面板会话").await.unwrap();
    db.append(&s.id, "todo.state", json!({"items":[{"text":"旧项"}]}))
        .await
        .unwrap();
    db.append(&s.id, "assistant.message", json!({"content":"中间消息"}))
        .await
        .unwrap();
    db.append(
        &s.id,
        "todo.state",
        json!({"items":[{"text":"新项","done":true}]}),
    )
    .await
    .unwrap();
    db.append(&s.id, "plan.state", json!({"content":"计划正文"}))
        .await
        .unwrap();
    db.append(&s.id, "goal.state", json!({"content":"目标 v1"}))
        .await
        .unwrap();
    db.append(&s.id, "goal.state", json!({"content":"目标 v2"}))
        .await
        .unwrap();

    let rows = db.latest_states().await.unwrap();
    assert_eq!(rows.len(), 3, "同会话三种 kind 各保留最新一条");
    let todo = rows.iter().find(|r| r.kind == "todo.state").unwrap();
    assert_eq!(todo.data["items"][0]["text"], "新项");
    assert_eq!(todo.session_id, s.id);
    assert_eq!(todo.session_title, "面板会话");
    assert_eq!(todo.workspace_id, "work-a");
    assert_eq!(
        rows.iter().find(|r| r.kind == "goal.state").unwrap().data["content"],
        "目标 v2"
    );
    assert_eq!(
        rows.iter().find(|r| r.kind == "plan.state").unwrap().data["content"],
        "计划正文"
    );
}

#[tokio::test]
async fn latest_states_skips_plain_sessions_and_orders_by_session_recency() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("events.db")).await.unwrap();
    let older = db.create_session("work-a", "较早会话").await.unwrap();
    db.append(&older.id, "todo.state", json!({"items":[]}))
        .await
        .unwrap();
    let plain = db.create_session("work-a", "无状态会话").await.unwrap();
    db.append(&plain.id, "user.message", json!({"content":"普通消息"}))
        .await
        .unwrap();
    let newer = db.create_session("work-b", "较新会话").await.unwrap();
    db.append(&newer.id, "goal.state", json!({"content":"新目标"}))
        .await
        .unwrap();

    let rows = db.latest_states().await.unwrap();
    assert_eq!(rows.len(), 2, "没有状态事件的会话不出现");
    assert_eq!(rows[0].session_id, newer.id, "最近有写入的会话排在最前");
    assert_eq!(rows[0].session_title, "较新会话");
    assert_eq!(rows[0].workspace_id, "work-b");
    assert_eq!(rows[1].session_id, older.id);
    assert!(rows.iter().all(|r| r.session_id != plain.id));
}
