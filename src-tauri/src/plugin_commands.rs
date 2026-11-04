//! Host-owned plugin forms and explicit commands; no guest JavaScript or HTML.
use harness_contracts::*;
use harness_runtime::Runtime;
use std::sync::Arc;
type Backend<'a> = tauri::State<'a, Arc<Runtime>>;
#[tauri::command]
#[specta::specta]
pub async fn plugin_contributions(
    state: Backend<'_>,
    workspace_id: String,
) -> AppResult<Vec<JsonPayload>> {
    Ok(state
        .plugin_contributions(&workspace_id)?
        .into_iter()
        .map(JsonPayload)
        .collect())
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_settings_save(
    state: Backend<'_>,
    workspace_id: String,
    plugin_id: String,
    values: JsonPayload,
) -> AppResult<()> {
    state
        .plugin_settings_save(&workspace_id, &plugin_id, values.0)
        .await
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_command(
    state: Backend<'_>,
    session_id: String,
    plugin_id: String,
    name: String,
    params: JsonPayload,
) -> AppResult<ToolResult> {
    state
        .plugin_command(&session_id, &plugin_id, &name, params.0)
        .await
}
#[tauri::command]
#[specta::specta]
pub async fn plugin_file_preview(
    state: Backend<'_>,
    workspace_id: String,
    path: String,
) -> AppResult<String> {
    state.plugin_file_preview(&workspace_id, &path)
}
