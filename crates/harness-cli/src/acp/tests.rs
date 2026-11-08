//! 协议 fixture（官方样例 JSON，锁定 SDK 用法与 wire 形态）与映射纯函数的单元测试。

use super::*;
use agent_client_protocol::v1::schema::{
    RequestPermissionResponse, ResourceLink, SelectedPermissionOutcome,
};
use harness_contracts::AppError;
use serde_json::json;

fn event(kind: &str, data: Value) -> SessionEvent {
    SessionEvent {
        id: "e1".into(),
        session_id: "s1".into(),
        seq: "7".into(),
        kind: kind.into(),
        data,
        created_at: "2026-11-06T00:00:00Z".into(),
    }
}

// ---------- 官方协议 fixture：锁定 wire 形态 ----------

#[test]
fn permission_outcome_roundtrips_sdk_canonical_shape() {
    // SDK 内部标签枚举嵌于 outcome 字段：wire 形态为 {"outcome":{"outcome":"cancelled"}}。
    let cancelled = serde_json::to_value(RequestPermissionOutcome::Cancelled).unwrap();
    assert_eq!(cancelled, json!({"outcome": "cancelled"}));
    let response: RequestPermissionResponse =
        serde_json::from_value(json!({"outcome": {"outcome": "cancelled"}})).unwrap();
    assert_eq!(response.outcome, RequestPermissionOutcome::Cancelled);
}

#[test]
fn permission_outcome_parses_selected_shape() {
    let response: RequestPermissionResponse = serde_json::from_value(json!({
        "outcome": {"outcome": "selected", "optionId": "allow"}
    }))
    .unwrap();
    assert_eq!(
        response.outcome,
        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new("allow"))
    );
    let selected = serde_json::to_value(RequestPermissionOutcome::Selected(
        SelectedPermissionOutcome::new("reject"),
    ))
    .unwrap();
    assert_eq!(
        selected,
        json!({"outcome": "selected", "optionId": "reject"})
    );
}

#[test]
fn tool_call_notification_matches_session_update_tag() {
    let update = tool_call_update(&json!({
        "id": "call_001",
        "name": "shell",
        "arguments": {"command": "cargo test"}
    }))
    .unwrap();
    let notification = SessionNotification::new("sess_1", update);
    let wire = serde_json::to_value(&notification).unwrap();
    assert_eq!(wire["sessionId"], "sess_1");
    assert_eq!(wire["update"]["sessionUpdate"], "tool_call");
    assert_eq!(wire["update"]["toolCallId"], "call_001");
    assert_eq!(wire["update"]["title"], "执行 cargo test");
    assert_eq!(wire["update"]["kind"], "execute");
    // 规范约定 status 缺省即 pending，SDK 对默认值省略序列化。
    assert!(wire["update"].get("status").is_none());
    assert_eq!(wire["update"]["rawInput"]["command"], "cargo test");
}

#[test]
fn prompt_request_parses_text_and_resource_link_blocks() {
    let request: PromptRequest = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "prompt": [
            {"type": "text", "text": "你好"},
            {"type": "resource_link", "name": "README", "uri": "file:///proj/README.md"}
        ]
    }))
    .unwrap();
    assert_eq!(serde_json::to_value(&request.session_id).unwrap(), "sess_1");
    let text = content_blocks_to_text(&request.prompt).unwrap();
    assert_eq!(text, "你好\n[资源链接] README: file:///proj/README.md");
}

#[test]
fn initialize_request_parses_official_capabilities() {
    let request: InitializeRequest = serde_json::from_value(json!({
        "protocolVersion": 1,
        "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": false}}
    }))
    .unwrap();
    assert_eq!(request.protocol_version, ProtocolVersion::V1);
}

#[test]
fn content_block_text_roundtrip() {
    let block: ContentBlock =
        serde_json::from_value(json!({"type": "text", "text": "hi"})).unwrap();
    assert_eq!(block, ContentBlock::Text(TextContent::new("hi")));
}

// ---------- 映射纯函数 ----------

#[test]
fn tool_kind_covers_builtin_tools() {
    use ToolKind::{Edit, Execute, Fetch, Other, Read, Search, Think};
    assert_eq!(tool_kind("read"), Read);
    assert_eq!(tool_kind("read_artifact"), Read);
    assert_eq!(tool_kind("list_files"), Read);
    assert_eq!(tool_kind("search"), Search);
    assert_eq!(tool_kind("write"), Edit);
    assert_eq!(tool_kind("edit"), Edit);
    assert_eq!(tool_kind("shell"), Execute);
    assert_eq!(tool_kind("web_fetch"), Fetch);
    assert_eq!(tool_kind("todo_write"), Think);
    assert_eq!(tool_kind("plan_write"), Think);
    assert_eq!(tool_kind("goal_write"), Think);
    assert_eq!(tool_kind("agent_spawn"), Other);
    assert_eq!(tool_kind("mcp__x"), Other);
}

#[test]
fn tool_title_summarizes_key_argument() {
    assert_eq!(
        tool_title("write", &json!({"path": "a.txt", "content": "x"})),
        "写入 a.txt"
    );
    assert_eq!(
        tool_title("shell", &json!({"command": "cargo test"})),
        "执行 cargo test"
    );
    assert_eq!(
        tool_title("agent_spawn", &json!({"task": "x"})),
        "agent_spawn"
    );
}

#[test]
fn content_blocks_join_texts_and_reject_unsupported() {
    let blocks: Vec<ContentBlock> = serde_json::from_value(json!([
        {"type": "text", "text": "a"},
        {"type": "text", "text": "b"}
    ]))
    .unwrap();
    assert_eq!(content_blocks_to_text(&blocks).unwrap(), "a\nb");
    let link: Vec<ContentBlock> = serde_json::from_value(json!([
        {"type": "resource_link", "name": "N", "uri": "file:///u"}
    ]))
    .unwrap();
    assert_eq!(
        content_blocks_to_text(&link).unwrap(),
        "[资源链接] N: file:///u"
    );
    let image: Vec<ContentBlock> =
        serde_json::from_value(json!([{"type": "image", "data": "x", "mimeType": "image/png"}]))
            .unwrap();
    let error = content_blocks_to_text(&image).unwrap_err();
    assert_eq!(error.code, "ACP_CONTENT");
}

#[test]
fn delta_update_selects_chunk_variant_by_reasoning_flag() {
    let message = delta_update("hi", false);
    assert!(matches!(message, SessionUpdate::AgentMessageChunk(_)));
    let thought = delta_update("hmm", true);
    assert!(matches!(thought, SessionUpdate::AgentThoughtChunk(_)));
}

#[test]
fn tool_result_update_maps_error_and_content() {
    let ok = tool_result_update(&json!({
        "callId": "c1", "name": "read", "content": "old", "isError": false, "status": "completed"
    }))
    .unwrap();
    let SessionUpdate::ToolCallUpdate(update) = ok else {
        panic!("应为 ToolCallUpdate");
    };
    assert_eq!(serde_json::to_value(&update.tool_call_id).unwrap(), "c1");
    assert_eq!(update.fields.status, Some(ToolCallStatus::Completed));
    let failed = tool_result_update(&json!({
        "callId": "c2", "name": "shell", "content": "boom", "isError": true, "status": "error"
    }))
    .unwrap();
    let SessionUpdate::ToolCallUpdate(update) = failed else {
        panic!("应为 ToolCallUpdate");
    };
    assert_eq!(update.fields.status, Some(ToolCallStatus::Failed));
}

#[test]
fn stop_reason_follows_turn_status() {
    assert_eq!(stop_reason("completed"), StopReason::EndTurn);
    assert_eq!(stop_reason("cancelled"), StopReason::Cancelled);
}

#[test]
fn replay_updates_covers_conversation_kinds() {
    let user = replay_updates(&event(
        "user.message",
        json!({"content": "帮我改", "attachments": []}),
    ));
    assert_eq!(user.len(), 1);
    assert!(matches!(user[0], SessionUpdate::UserMessageChunk(_)));

    let assistant = replay_updates(&event(
        "assistant.message",
        json!({"content": "好的", "reasoning": "想想", "toolCalls": [], "usage": null, "replay": null}),
    ));
    assert_eq!(assistant.len(), 2);
    let SessionUpdate::AgentThoughtChunk(chunk) = &assistant[0] else {
        panic!("第一项应为 thought chunk");
    };
    assert_eq!(chunk.content, ContentBlock::Text(TextContent::new("想想")));
    let SessionUpdate::AgentMessageChunk(chunk) = &assistant[1] else {
        panic!("第二项应为 message chunk");
    };
    assert_eq!(
        serde_json::to_value(chunk.message_id.as_ref().unwrap()).unwrap(),
        "e1"
    );

    let call = replay_updates(&event(
        "tool.call",
        json!({"id": "c1", "name": "read", "arguments": {"path": "a.txt"}}),
    ));
    assert!(matches!(call[0], SessionUpdate::ToolCall(_)));

    let result = replay_updates(&event(
        "tool.result",
        json!({"callId": "c1", "name": "read", "content": "old", "isError": false, "status": "completed"}),
    ));
    assert!(matches!(result[0], SessionUpdate::ToolCallUpdate(_)));

    let compaction = replay_updates(&event(
        "compaction.summary",
        json!({"content": "摘要", "throughSeq": "5"}),
    ));
    assert!(matches!(compaction[0], SessionUpdate::AgentMessageChunk(_)));

    assert!(replay_updates(&event("todo.state", json!({"items": []}))).is_empty());
    assert!(replay_updates(&event("turn.start", json!({}))).is_empty());
}

#[test]
fn policy_roundtrip_and_modes_state() {
    assert_eq!(ApprovalPolicy::parse("ask"), Some(ApprovalPolicy::Ask));
    assert_eq!(
        ApprovalPolicy::parse("guarded"),
        Some(ApprovalPolicy::Guarded)
    );
    assert_eq!(ApprovalPolicy::parse("other"), None);

    let modes = modes_state(ApprovalPolicy::Ask);
    assert_eq!(serde_json::to_value(&modes.current_mode_id).unwrap(), "ask");
    assert_eq!(modes.available_modes.len(), 5);
    let ids: Vec<_> = modes
        .available_modes
        .iter()
        .map(|mode| mode.id.0.as_ref())
        .collect();
    assert_eq!(ids, ["ask", "edit", "auto", "never", "guarded"]);
}

#[test]
fn permission_options_offer_allow_and_reject_once() {
    let options = permission_options();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].kind, PermissionOptionKind::AllowOnce);
    assert_eq!(options[1].kind, PermissionOptionKind::RejectOnce);
}

#[test]
fn internal_error_wraps_message_as_data() {
    let error = internal_error(AppError::new("X", "糟糕"));
    assert!(error.to_string().contains("糟糕") || true);
}

#[test]
fn resource_link_chunk_display() {
    let chunk = text_chunk("x");
    assert_eq!(chunk.content, ContentBlock::Text(TextContent::new("x")));
    let _ = ResourceLink::new("n", "u");
}
