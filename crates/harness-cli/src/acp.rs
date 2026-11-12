//! ACP（Agent Client Protocol）v1 agent 端：编辑器客户端经 stdio 驱动本 Agent。
//!
//! 协议类型与 ndjson 帧处理来自官方 SDK `agent-client-protocol`；本模块负责
//! 协议到运行时的映射：会话生命周期、事件流转发、审批转交互式授权与历史回放。
//! 详见 `docs/specs/acp.md`。

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, CurrentModeUpdate,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionId, SessionMode,
    SessionModeId, SessionModeState, SessionNotification, SessionUpdate, SetSessionModeRequest,
    SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallId, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{ConnectTo, on_receive_notification, on_receive_request};
use harness_contracts::{AppError, AppResult, ApprovalPolicy, LiveEvent, SessionEvent};
use harness_runtime::Runtime;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// `harness acp` 的启动参数。
#[derive(Debug, Default, Clone)]
pub struct AcpOptions {
    /// 模型 ID；缺省时回落到 settings `cli.default_model`。
    pub model: Option<String>,
    /// 能力预设；缺省时回落到 settings `cli.default_preset`，显式 `--model` 优先于预设字段。
    pub preset: Option<String>,
}

/// ACP 审批策略即 contracts 五档权限模式；`session/set_mode` 的 mode id 为其字符串形式。
fn modes_state(current: ApprovalPolicy) -> SessionModeState {
    SessionModeState::new(
        current.as_str(),
        [
            (ApprovalPolicy::Ask, "每次确认"),
            (ApprovalPolicy::Edit, "自动编辑"),
            (ApprovalPolicy::Auto, "完全访问"),
            (ApprovalPolicy::Never, "只读"),
            (ApprovalPolicy::Guarded, "自主"),
        ]
        .into_iter()
        .map(|(policy, name)| SessionMode::new(policy.as_str(), name))
        .collect(),
    )
}

/// 工具调用在 ACP 端的粗分类（图标/UI 选择用）。
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "read" | "read_artifact" | "list_files" => ToolKind::Read,
        "search" => ToolKind::Search,
        "write" | "edit" => ToolKind::Edit,
        "shell" => ToolKind::Execute,
        "web_fetch" => ToolKind::Fetch,
        "todo_write" | "plan_write" | "goal_write" | "ask_user" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

/// 工具调用的人类可读标题。
fn tool_title(name: &str, args: &Value) -> String {
    let field = |key: &str| args[key].as_str().unwrap_or_default();
    let hint = match name {
        "write" => format!("写入 {}", field("path")),
        "edit" => format!("编辑 {}", field("path")),
        "shell" => format!("执行 {}", field("command")),
        "web_fetch" => format!("获取 {}", field("url")),
        "search" => format!("搜索 {}", field("query")),
        "read" | "read_artifact" | "list_files" => format!("读取 {}", field("path")),
        "ask_user" => format!("提问 {}", field("question")),
        _ => String::new(),
    };
    if hint.is_empty() {
        name.to_owned()
    } else {
        hint.chars().take(120).collect()
    }
}

/// ACP 提示内容块拼接为文本；仅支持 text 与 resource_link（按能力声明）。
fn content_blocks_to_text(blocks: &[ContentBlock]) -> AppResult<String> {
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text(text) => parts.push(text.text.clone()),
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[资源链接] {}: {}", link.name, link.uri));
            }
            _ => {
                return Err(AppError::new(
                    "ACP_CONTENT",
                    "当前仅支持 text 与 resource_link 内容块",
                ));
            }
        }
    }
    Ok(parts.join("\n"))
}

fn text_chunk(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

/// 流式增量 → ACP 更新。
fn delta_update(text: &str, reasoning: bool) -> SessionUpdate {
    if reasoning {
        SessionUpdate::AgentThoughtChunk(text_chunk(text))
    } else {
        SessionUpdate::AgentMessageChunk(text_chunk(text))
    }
}

/// `tool.call` 持久事件 → `tool_call` 初始通知。
fn tool_call_update(data: &Value) -> Option<SessionUpdate> {
    let id = data["id"].as_str()?;
    let name = data["name"].as_str().unwrap_or_default();
    Some(SessionUpdate::ToolCall(
        ToolCall::new(id.to_owned(), tool_title(name, &data["arguments"]))
            .name(name)
            .kind(tool_kind(name))
            .status(ToolCallStatus::Pending)
            .raw_input(data["arguments"].clone()),
    ))
}

/// `tool.result` 持久事件 → `tool_call_update` 终态通知。
fn tool_result_update(data: &Value) -> Option<SessionUpdate> {
    let call_id = data["callId"].as_str()?;
    let status = if data["isError"].as_bool() == Some(true) {
        ToolCallStatus::Failed
    } else {
        ToolCallStatus::Completed
    };
    let content = data["content"].as_str().unwrap_or_default();
    Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        call_id.to_owned(),
        ToolCallUpdateFields::new()
            .status(status)
            .content(vec![ContentBlock::Text(TextContent::new(content)).into()]),
    )))
}

/// 审批交互选项：允许一次 / 拒绝一次。
fn permission_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption::new("allow", "允许", PermissionOptionKind::AllowOnce),
        PermissionOption::new("reject", "拒绝", PermissionOptionKind::RejectOnce),
    ]
}

/// 回合终态 → stopReason。
fn stop_reason(status: &str) -> StopReason {
    if status == "cancelled" {
        StopReason::Cancelled
    } else {
        StopReason::EndTurn
    }
}

/// 持久事件 → 历史回放更新（`session/load` 与 Lagged 补拉共用）。
fn replay_updates(event: &SessionEvent) -> Vec<SessionUpdate> {
    match event.kind.as_str() {
        "user.message" => event.data["content"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| vec![SessionUpdate::UserMessageChunk(text_chunk(text))])
            .unwrap_or_default(),
        "assistant.message" => {
            let mut updates = Vec::new();
            if let Some(reasoning) = event.data["reasoning"].as_str().filter(|s| !s.is_empty()) {
                updates.push(SessionUpdate::AgentThoughtChunk(text_chunk(reasoning)));
            }
            if let Some(content) = event.data["content"].as_str().filter(|s| !s.is_empty()) {
                let chunk = text_chunk(content).message_id(event.id.as_str());
                updates.push(SessionUpdate::AgentMessageChunk(chunk));
            }
            updates
        }
        "tool.call" => tool_call_update(&event.data).into_iter().collect(),
        "tool.result" => tool_result_update(&event.data).into_iter().collect(),
        "compaction.summary" => event.data["content"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| vec![SessionUpdate::AgentMessageChunk(text_chunk(text))])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn internal_error(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(e.to_string())
}

/// 连接级共享状态。
struct AcpState {
    runtime: Arc<Runtime>,
    model: Option<String>,
    preset: Option<harness_runtime::presets::ResolvedPreset>,
    initialized: AtomicBool,
    /// ACP 会话（id = 内部会话 UUID）→ 工作区与审批策略。
    sessions: Mutex<BTreeMap<String, (String, ApprovalPolicy)>>,
    /// 每会话最近一次 pending 的工具调用 id（审批请求关联用）。
    pending_calls: Mutex<BTreeMap<String, String>>,
    /// 每会话已转发的最大持久事件 seq（Lagged 补拉游标）。
    seqs: Mutex<BTreeMap<String, u64>>,
}

impl AcpState {
    /// 新会话的初始审批策略：预设档位优先，缺省 ask；`session/set_mode` 仍可切换。
    fn initial_policy(&self) -> ApprovalPolicy {
        self.preset
            .as_ref()
            .and_then(|resolved| resolved.approval.as_deref())
            .and_then(ApprovalPolicy::parse)
            .unwrap_or(ApprovalPolicy::Ask)
    }

    fn known(&self, session: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(session)
    }
    fn policy(&self, session: &str) -> ApprovalPolicy {
        self.sessions
            .lock()
            .unwrap()
            .get(session)
            .map(|(_, policy)| *policy)
            .unwrap_or(ApprovalPolicy::Ask)
    }
    fn touch(&self, event: &SessionEvent) {
        if let Ok(seq) = event.seq.parse::<u64>() {
            let mut seqs = self.seqs.lock().unwrap();
            let last = seqs.entry(event.session_id.clone()).or_default();
            if seq > *last {
                *last = seq;
            }
        }
    }
}

pub async fn serve(runtime: Arc<Runtime>, options: AcpOptions) -> AppResult<()> {
    serve_transport(runtime, options, agent_client_protocol::Stdio::new()).await
}

/// 经任意传输运行 ACP agent（测试入口）。
pub async fn serve_transport(
    runtime: Arc<Runtime>,
    options: AcpOptions,
    transport: impl ConnectTo<agent_client_protocol::Agent>,
) -> AppResult<()> {
    // ACP v1 未实现 elicitation 交互转发（docs/specs/acp.md 非目标）：
    // ask_user 提问以无人在场自动结算，避免编辑器客户端下回合无限挂起。
    runtime.set_unattended(true);
    // 预设启动时解析一次，未知 ID 立即失败而不是在每个回合报错。
    let preset = crate::manage::resolve_preset(&runtime, options.preset.as_deref()).await?;
    let state = Arc::new(AcpState {
        preset,
        runtime,
        model: options.model,
        initialized: AtomicBool::new(false),
        sessions: Mutex::new(BTreeMap::new()),
        pending_calls: Mutex::new(BTreeMap::new()),
        seqs: Mutex::new(BTreeMap::new()),
    });
    run_connection(state, transport)
        .await
        .map_err(|e| AppError::new("ACP", e.to_string()))
}

async fn run_connection(
    state: Arc<AcpState>,
    transport: impl ConnectTo<agent_client_protocol::Agent>,
) -> Result<(), agent_client_protocol::Error> {
    let init = state.clone();
    let new_session = state.clone();
    let load = state.clone();
    let prompt = state.clone();
    let mode = state.clone();
    let cancel = state.clone();
    agent_client_protocol::Agent
        .v1()
        .on_receive_request(
            async move |_request: InitializeRequest, responder, _cx| {
                init.initialized.store(true, Ordering::SeqCst);
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        .agent_capabilities(AgentCapabilities::new().load_session(true)),
                )
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _cx| {
                if !new_session.initialized.load(Ordering::SeqCst) {
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_request().data("请先 initialize"),
                    );
                }
                if !request.mcp_servers.is_empty() {
                    tracing::warn!("ACP 客户端请求的 mcpServers 暂不支持，已忽略");
                }
                let workspace = match new_session.runtime.workspaces.create(
                    &request.cwd,
                    None,
                    harness_contracts::ExecutionMode::Replica,
                ) {
                    Ok(workspace) => workspace,
                    Err(e) => {
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params().data(e.message),
                        );
                    }
                };
                let title = request
                    .cwd
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "新会话".into());
                let session = match new_session
                    .runtime
                    .store
                    .create_session(&workspace.id, &title)
                    .await
                {
                    Ok(session) => session,
                    Err(e) => return responder.respond_with_error(internal_error(e)),
                };
                let initial = new_session.initial_policy();
                new_session
                    .sessions
                    .lock()
                    .unwrap()
                    .insert(session.id.clone(), (workspace.id, initial));
                responder.respond(
                    NewSessionResponse::new(SessionId::new(session.id.as_str()))
                        .modes(modes_state(initial)),
                )
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, cx| {
                if !load.initialized.load(Ordering::SeqCst) {
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_request().data("请先 initialize"),
                    );
                }
                let session_id = request.session_id.0.to_string();
                let info = match load.runtime.store.session(&session_id).await {
                    Ok(info) => info,
                    Err(e) => {
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params().data(e.message),
                        );
                    }
                };
                load.sessions
                    .lock()
                    .unwrap()
                    .insert(session_id.clone(), (info.workspace_id, ApprovalPolicy::Ask));
                let events = match load.runtime.store.events(&session_id, 0).await {
                    Ok(events) => events,
                    Err(e) => return responder.respond_with_error(internal_error(e)),
                };
                for event in &events {
                    load.touch(event);
                    for update in replay_updates(event) {
                        cx.send_notification(SessionNotification::new(
                            SessionId::new(session_id.as_str()),
                            update,
                        ))?;
                    }
                }
                responder
                    .respond(LoadSessionResponse::new().modes(modes_state(ApprovalPolicy::Ask)))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx| {
                if !prompt.initialized.load(Ordering::SeqCst) {
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_request().data("请先 initialize"),
                    );
                }
                let session_id = request.session_id.0.to_string();
                if !prompt.known(&session_id) {
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_params().data("未知会话"),
                    );
                }
                let text = match content_blocks_to_text(&request.prompt) {
                    Ok(text) => text,
                    Err(e) => {
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params().data(e.message),
                        );
                    }
                };
                let runtime = prompt.runtime.clone();
                let explicit = prompt.model.clone();
                let preset = prompt.preset.clone();
                // 长回合移出 dispatch loop，保证 session/cancel 可达；responder 移入延迟应答。
                cx.spawn(async move {
                    let outcome = run_turn(runtime, &session_id, text, explicit, preset).await;
                    match outcome {
                        Ok(status) => responder.respond(PromptResponse::new(stop_reason(&status))),
                        // 取消经模型调用错误返回，按规范必须应答 cancelled。
                        Err(e) if e.code == "CANCELLED" => {
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        }
                        Err(e) => responder.respond_with_error(agent_client_protocol::Error::new(
                            agent_client_protocol::ErrorCode::INTERNAL_ERROR,
                            e.message,
                        )),
                    }
                })?;
                Ok(())
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, cx| {
                let session_id = request.session_id.0.to_string();
                let Some(policy) = ApprovalPolicy::parse(&request.mode_id.0) else {
                    return responder.respond_with_error(
                        agent_client_protocol::Error::invalid_params().data("未知模式"),
                    );
                };
                let mut sessions = mode.sessions.lock().unwrap();
                match sessions.get_mut(&session_id) {
                    Some(entry) => entry.1 = policy,
                    None => {
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params().data("未知会话"),
                        );
                    }
                }
                drop(sessions);
                cx.send_notification(SessionNotification::new(
                    SessionId::new(session_id.as_str()),
                    SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(SessionModeId::new(
                        policy.as_str(),
                    ))),
                ))?;
                responder.respond(SetSessionModeResponse::new())
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _cx| {
                cancel.runtime.cancel(&notification.session_id.0);
                Ok(())
            },
            on_receive_notification!(),
        )
        .connect_with(transport, async move |cx| {
            let pump_state = state.clone();
            let pump_cx = cx.clone();
            cx.spawn(async move { pump(pump_cx, pump_state).await })?;
            cx.incoming_closed().await;
            state.runtime.cancel_all();
            Ok(())
        })
        .await
}

/// 执行一个回合并返回终态 status（completed/cancelled）。
async fn run_turn(
    runtime: Arc<Runtime>,
    session_id: &str,
    text: String,
    explicit_model: Option<String>,
    preset: Option<harness_runtime::presets::ResolvedPreset>,
) -> AppResult<String> {
    // 分层：--model > 预设 modelId > cli.default_model；effort 取预设档位（显式参数无对应项）。
    let model_id = crate::manage::resolve_model_id(
        &runtime,
        explicit_model.or_else(|| crate::manage::preset_model(&preset)),
    )
    .await?;
    let model = runtime
        .models
        .build_model_with(&model_id, crate::manage::preset_effort(&preset))
        .await?;
    let mut events = runtime.subscribe();
    runtime
        .run_session_with_attachments(session_id.to_owned(), text, model, Vec::new())
        .await?;
    // run_session 返回时 turn.end/Status 已广播，扫订阅缓冲取终态。
    let mut status = "completed".to_owned();
    while let Ok(event) = events.try_recv() {
        if let LiveEvent::Status {
            session_id: sid,
            status: end,
        } = event
            && sid == session_id
        {
            status = end;
        }
    }
    Ok(status)
}

/// 事件泵：把运行时事件映射为 session/update 通知并驱动审批交互。
async fn pump(
    cx: agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Client>,
    state: Arc<AcpState>,
) -> Result<(), agent_client_protocol::Error> {
    use tokio::sync::broadcast::error::RecvError;
    let mut events = state.runtime.subscribe();
    loop {
        match events.recv().await {
            Ok(LiveEvent::Delta {
                session_id,
                text,
                reasoning,
                ..
            }) => {
                if text.is_empty() || !state.known(&session_id) {
                    continue;
                }
                cx.send_notification(SessionNotification::new(
                    SessionId::new(session_id.as_str()),
                    delta_update(&text, reasoning),
                ))?;
            }
            Ok(LiveEvent::Durable { event }) => {
                if !state.known(&event.session_id) {
                    continue;
                }
                state.touch(&event);
                match event.kind.as_str() {
                    "tool.call" => {
                        if let Some(id) = event.data["id"].as_str() {
                            state
                                .pending_calls
                                .lock()
                                .unwrap()
                                .insert(event.session_id.clone(), id.to_owned());
                        }
                        if let Some(update) = tool_call_update(&event.data) {
                            cx.send_notification(SessionNotification::new(
                                SessionId::new(event.session_id.as_str()),
                                update,
                            ))?;
                        }
                    }
                    "tool.result" => {
                        if let Some(update) = tool_result_update(&event.data) {
                            cx.send_notification(SessionNotification::new(
                                SessionId::new(event.session_id.as_str()),
                                update,
                            ))?;
                        }
                    }
                    "approval.request" => handle_approval(&cx, &state, &event).await?,
                    _ => {}
                }
            }
            Ok(LiveEvent::Status { .. }) => {}
            Err(RecvError::Lagged(_)) => resync(&cx, &state).await?,
            Err(RecvError::Closed) => return Ok(()),
        }
    }
}

/// 审批：ask 策略转交互式授权请求，auto/never 直接结算。
async fn handle_approval(
    cx: &agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Client>,
    state: &Arc<AcpState>,
    event: &SessionEvent,
) -> Result<(), agent_client_protocol::Error> {
    let request: harness_contracts::ApprovalRequest =
        serde_json::from_value(event.data.clone()).map_err(internal_error)?;
    let session_id = request.session_id.clone();
    // 决策矩阵统一在 contracts::ApprovalPolicy::decide；None 才发 request_permission。
    match state.policy(&session_id).decide(&request) {
        Some(allow) => {
            let _ = state.runtime.approvals.resolve(&request.id, allow);
            Ok(())
        }
        None => {
            let tool_call_id = state
                .pending_calls
                .lock()
                .unwrap()
                .get(&session_id)
                .cloned()
                .unwrap_or_else(|| request.id.clone());
            let response = cx
                .send_request(RequestPermissionRequest::new(
                    SessionId::new(session_id.as_str()),
                    ToolCallUpdate::new(
                        ToolCallId::new(tool_call_id.as_str()),
                        ToolCallUpdateFields::new(),
                    ),
                    permission_options(),
                ))
                .block_task()
                .await?;
            let allow = match response.outcome {
                RequestPermissionOutcome::Cancelled => false,
                RequestPermissionOutcome::Selected(selected) => {
                    selected.option_id.0.as_ref() == "allow"
                }
                _ => false,
            };
            let _ = state.runtime.approvals.resolve(&request.id, allow);
            Ok(())
        }
    }
}

/// broadcast 滞后时按 seq 游标从存储补拉持久事件。
async fn resync(
    cx: &agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Client>,
    state: &Arc<AcpState>,
) -> Result<(), agent_client_protocol::Error> {
    let snapshot: Vec<(String, u64)> = state.seqs.lock().unwrap().clone().into_iter().collect();
    for (session_id, after) in snapshot {
        let events = state
            .runtime
            .store
            .events(&session_id, after)
            .await
            .map_err(internal_error)?;
        for event in events {
            state.touch(&event);
            for update in replay_updates(&event) {
                cx.send_notification(SessionNotification::new(
                    SessionId::new(session_id.as_str()),
                    update,
                ))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
