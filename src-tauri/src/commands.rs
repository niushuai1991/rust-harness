//! Typed desktop adapters. Agent tools never call these human-only synchronization endpoints.
use harness_contracts::*;
use harness_runtime::Runtime;
use std::{path::PathBuf, sync::Arc};
use tauri::{State, ipc::Channel};

type Backend<'a> = State<'a, Arc<Runtime>>;
/// 跨重启保留的上次模型选择（settings KV 键）。
pub const DEFAULT_SELECTION_KEY: &str = "app.default_selection";
fn error(e: impl std::fmt::Display) -> AppError {
    AppError::new("DESKTOP", e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_list(state: Backend<'_>) -> AppResult<Vec<WorkspaceInfo>> {
    state.workspaces.list()
}
#[tauri::command]
#[specta::specta]
pub async fn workspace_open(state: Backend<'_>, path: String) -> AppResult<WorkspaceInfo> {
    let manager = state.workspaces.clone();
    tokio::task::spawn_blocking(move || manager.create(&PathBuf::from(path)))
        .await
        .map_err(error)?
}
#[tauri::command]
#[specta::specta]
pub async fn workspace_diff(
    state: Backend<'_>,
    workspace_id: String,
) -> AppResult<Vec<FileChange>> {
    let manager = state.workspaces.clone();
    tokio::task::spawn_blocking(move || manager.diff(&workspace_id))
        .await
        .map_err(error)?
}
#[tauri::command]
#[specta::specta]
pub async fn workspace_apply(
    state: Backend<'_>,
    workspace_id: String,
    paths: Vec<String>,
    reviewed: Vec<FileChange>,
) -> AppResult<Vec<FileChange>> {
    if paths.len() != reviewed.len() || paths.iter().any(|p| !reviewed.iter().any(|r| &r.path == p))
    {
        return Err(AppError::new("STALE_DIFF", "确认路径与差异预览不一致"));
    }
    let manager = state.workspaces.clone();
    tokio::task::spawn_blocking(move || manager.apply_reviewed(&workspace_id, &reviewed))
        .await
        .map_err(error)?
}
#[tauri::command]
#[specta::specta]
pub async fn workspace_refresh(
    state: Backend<'_>,
    workspace_id: String,
) -> AppResult<WorkspaceInfo> {
    let manager = state.workspaces.clone();
    tokio::task::spawn_blocking(move || manager.refresh(&workspace_id))
        .await
        .map_err(error)?
}
#[tauri::command]
#[specta::specta]
pub async fn session_list(state: Backend<'_>, workspace_id: String) -> AppResult<Vec<SessionInfo>> {
    state.list_sessions(Some(&workspace_id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn session_active_list(state: Backend<'_>) -> AppResult<Vec<String>> {
    Ok(state.active_ids())
}
#[tauri::command]
#[specta::specta]
pub async fn session_create(
    state: Backend<'_>,
    workspace_id: String,
    title: String,
) -> AppResult<SessionInfo> {
    state.workspaces.get(&workspace_id)?;
    state.store.create_session(&workspace_id, &title).await
}
#[tauri::command]
#[specta::specta]
pub async fn session_events(
    state: Backend<'_>,
    session_id: String,
    after: Option<String>,
) -> AppResult<Vec<SessionEvent>> {
    let after = after
        .unwrap_or_else(|| "0".into())
        .parse::<u64>()
        .map_err(error)?;
    state.store.events(&session_id, after).await
}
#[tauri::command]
#[specta::specta]
pub async fn session_send(
    state: Backend<'_>,
    session_id: String,
    text: String,
    model_id: String,
    effort: Option<String>,
    on_event: Channel<LiveEvent>,
    attachments: Option<Vec<String>>,
) -> AppResult<()> {
    let provider = state.models.build_model_with(&model_id, effort).await?;
    let runtime = state.inner().clone();
    let mut live = runtime.subscribe();
    let sid = session_id.clone();
    let forward = tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(event) => {
                    let target = match &event {
                        LiveEvent::Durable { event } => &event.session_id,
                        LiveEvent::Delta { session_id, .. }
                        | LiveEvent::Status { session_id, .. } => session_id,
                    };
                    if target == &sid && on_event.send(event).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => break,
                Err(_) => break,
            }
        }
    });
    let result = runtime
        .run_session_with_attachments(session_id, text, provider, attachments.unwrap_or_default())
        .await;
    // Allow the final durable frame to drain; frontend also performs cursor-safe history reload.
    tokio::task::yield_now().await;
    forward.abort();
    result
}
#[tauri::command]
#[specta::specta]
pub async fn session_cancel(state: Backend<'_>, session_id: String) -> AppResult<()> {
    state.cancel(&session_id);
    Ok(())
}
#[tauri::command]
#[specta::specta]
pub async fn session_export(state: Backend<'_>, session_id: String, path: String) -> AppResult<()> {
    let content = state.store.export(&session_id).await?;
    tokio::fs::write(path, content).await.map_err(error)
}
#[tauri::command]
#[specta::specta]
pub async fn session_fork(state: Backend<'_>, session_id: String) -> AppResult<SessionInfo> {
    state.fork(&session_id).await
}
#[tauri::command]
#[specta::specta]
pub async fn session_search(state: Backend<'_>, query: String) -> AppResult<Vec<SessionInfo>> {
    state.store.search(&query).await
}
/// 压缩会话历史为摘要（与 CLI `/compact`、JSON-RPC `session.compact` 同一路径）。
#[tauri::command]
#[specta::specta]
pub async fn session_compact(
    state: Backend<'_>,
    session_id: String,
    model_id: String,
) -> AppResult<()> {
    let provider = state.models.build_model(&model_id).await?;
    harness_orchestration::compact_session(
        state.inner().clone(),
        &session_id,
        provider,
        Default::default(),
    )
    .await
}
#[tauri::command]
#[specta::specta]
pub async fn plan_overview(state: Backend<'_>) -> AppResult<Vec<PlanStateOverview>> {
    state.store.latest_states().await
}

#[tauri::command]
#[specta::specta]
pub async fn provider_list(state: Backend<'_>) -> AppResult<Vec<ProviderInfo>> {
    state.models.list_providers().await
}
#[tauri::command]
#[specta::specta]
pub async fn provider_save(
    state: Backend<'_>,
    config: ProviderInfo,
    secret: Option<String>,
) -> AppResult<ProviderInfo> {
    state.models.save_provider(config, secret).await
}
#[tauri::command]
#[specta::specta]
pub async fn provider_copy(state: Backend<'_>, id: String) -> AppResult<ProviderInfo> {
    state.models.copy_provider(&id).await
}
#[tauri::command]
#[specta::specta]
pub async fn provider_remove(state: Backend<'_>, id: String) -> AppResult<()> {
    state.models.remove_provider(&id).await
}
#[tauri::command]
#[specta::specta]
pub async fn model_list(state: Backend<'_>) -> AppResult<Vec<ModelConfig>> {
    state.models.list_models().await
}
#[tauri::command]
#[specta::specta]
pub async fn model_save(
    state: Backend<'_>,
    config: ModelConfig,
    secret: Option<String>,
) -> AppResult<()> {
    state.models.save_model(config, secret).await?;
    Ok(())
}
#[tauri::command]
#[specta::specta]
pub async fn model_remove(state: Backend<'_>, model_id: String) -> AppResult<()> {
    state.models.remove_model(&model_id).await
}
#[tauri::command]
#[specta::specta]
pub async fn model_test(state: Backend<'_>, model_id: String) -> AppResult<String> {
    state.models.test_model(&model_id).await?;
    Ok("连接成功".into())
}
#[tauri::command]
#[specta::specta]
pub async fn model_fetch_available(
    state: Backend<'_>,
    provider_id: String,
    api_mode: ApiMode,
) -> AppResult<Vec<String>> {
    state
        .models
        .fetch_available_models(&provider_id, api_mode)
        .await
}
#[tauri::command]
#[specta::specta]
pub async fn default_selection_get(state: Backend<'_>) -> AppResult<Option<DefaultSelection>> {
    state
        .store
        .get(DEFAULT_SELECTION_KEY)
        .await?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| AppError::new("MODEL_CONFIG", e.to_string()))
}
#[tauri::command]
#[specta::specta]
pub async fn default_selection_save(
    state: Backend<'_>,
    selection: DefaultSelection,
) -> AppResult<()> {
    let value = serde_json::to_value(&selection)
        .map_err(|e| AppError::new("MODEL_CONFIG", e.to_string()))?;
    state.store.set(DEFAULT_SELECTION_KEY, &value).await
}
#[tauri::command]
#[specta::specta]
pub async fn approval_list(state: Backend<'_>) -> AppResult<Vec<ApprovalRequest>> {
    Ok(state.approvals.list())
}
#[tauri::command]
#[specta::specta]
pub async fn approval_resolve(state: Backend<'_>, id: String, allow: bool) -> AppResult<()> {
    state.approvals.resolve(&id, allow)
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_list(state: Backend<'_>) -> AppResult<Vec<PluginInfo>> {
    state.plugins.list()
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_install(
    state: Backend<'_>,
    path: String,
    permissions: Vec<String>,
) -> AppResult<PluginInfo> {
    let plugins = state.plugins.clone();
    tokio::task::spawn_blocking(move || plugins.install(&PathBuf::from(path), permissions))
        .await
        .map_err(error)?
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_enable(state: Backend<'_>, id: String, enabled: bool) -> AppResult<()> {
    state.plugins.set_enabled(&id, enabled)
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_remove(state: Backend<'_>, id: String) -> AppResult<()> {
    state.plugins.remove(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_list(state: Backend<'_>) -> AppResult<Vec<JsonPayload>> {
    Ok(state
        .mcp_connections()
        .into_iter()
        .map(JsonPayload)
        .collect())
}
#[tauri::command]
#[specta::specta]
pub async fn mcp_connect(
    state: Backend<'_>,
    workspace_id: String,
    config: JsonPayload,
) -> AppResult<()> {
    let config = serde_json::from_value(config.0).map_err(error)?;
    state.connect_mcp(&workspace_id, config).await
}
#[tauri::command]
#[specta::specta]
pub async fn mcp_disconnect(state: Backend<'_>, id: String) -> AppResult<()> {
    state.disconnect_mcp(&id).await
}
#[tauri::command]
#[specta::specta]
pub async fn mcp_resources(
    state: Backend<'_>,
    id: String,
    uri: Option<String>,
) -> AppResult<JsonPayload> {
    match uri {
        Some(uri) => state.mcp.read_resource(&id, &uri).await.map(JsonPayload),
        None => state.mcp.list_resources(&id).await.map(JsonPayload),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn lsp_status(
    state: Backend<'_>,
    workspace_id: Option<String>,
) -> AppResult<Vec<LspStatusEntry>> {
    let status = state.lsp_status().await;
    Ok(match workspace_id {
        Some(id) => status
            .into_iter()
            .filter(|e| e.workspace_id == id)
            .collect(),
        None => status,
    })
}
#[tauri::command]
#[specta::specta]
pub async fn lsp_restart(
    state: Backend<'_>,
    workspace_id: String,
    server_id: String,
) -> AppResult<()> {
    state.lsp_restart(&workspace_id, &server_id).await
}

type Orchestration<'a> = State<'a, harness_orchestration::Installed>;
fn payload(value: impl serde::Serialize) -> AppResult<JsonPayload> {
    serde_json::to_value(value).map(JsonPayload).map_err(error)
}
#[tauri::command]
#[specta::specta]
pub async fn job_list(orch: Orchestration<'_>, session_id: String) -> AppResult<Vec<JsonPayload>> {
    orch.session_jobs(&session_id)
        .await?
        .into_iter()
        .map(payload)
        .collect()
}
#[tauri::command]
#[specta::specta]
pub async fn job_cancel(
    orch: Orchestration<'_>,
    session_id: String,
    job_id: String,
) -> AppResult<()> {
    orch.cancel_job(&session_id, &job_id).await
}
#[tauri::command]
#[specta::specta]
pub async fn schedule_list(
    orch: Orchestration<'_>,
    session_id: String,
) -> AppResult<Vec<JsonPayload>> {
    orch.session_schedules(&session_id)
        .await?
        .into_iter()
        .map(payload)
        .collect()
}
#[tauri::command]
#[specta::specta]
pub async fn schedule_delete(
    orch: Orchestration<'_>,
    session_id: String,
    id: String,
) -> AppResult<()> {
    orch.delete_schedule(&session_id, &id).await
}

pub fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new().commands(tauri_specta::collect_commands![
        workspace_list,
        workspace_open,
        workspace_diff,
        workspace_apply,
        workspace_refresh,
        session_list,
        session_active_list,
        session_create,
        session_events,
        session_send,
        session_cancel,
        session_export,
        session_fork,
        session_search,
        session_compact,
        plan_overview,
        provider_list,
        provider_save,
        provider_copy,
        provider_remove,
        model_remove,
        model_list,
        model_save,
        model_test,
        model_fetch_available,
        default_selection_get,
        default_selection_save,
        approval_list,
        approval_resolve,
        plugin_list,
        plugin_install,
        plugin_enable,
        plugin_remove,
        mcp_list,
        mcp_connect,
        mcp_disconnect,
        mcp_resources,
        lsp_status,
        lsp_restart,
        job_list,
        job_cancel,
        schedule_list,
        schedule_delete,
        crate::artifact_commands::artifact_import,
        crate::artifact_commands::artifact_read,
        crate::artifact_commands::artifact_save,
        crate::plugin_commands::plugin_contributions,
        crate::plugin_commands::plugin_settings_save,
        crate::plugin_commands::plugin_command,
        crate::plugin_commands::plugin_file_preview
    ])
}
