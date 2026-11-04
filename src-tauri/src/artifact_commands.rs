//! Human-selected import/export adapters; models can only read scoped artifact chunks.
use harness_contracts::{AppResult, ArtifactChunk, ArtifactMetadata};
use harness_runtime::Runtime;
use std::{path::PathBuf, sync::Arc};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn artifact_import(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    path: String,
) -> AppResult<ArtifactMetadata> {
    state
        .artifact_import(&session_id, &PathBuf::from(path))
        .await
}
#[tauri::command]
#[specta::specta]
pub async fn artifact_read(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    hash: String,
    offset: u32,
    length: u32,
) -> AppResult<ArtifactChunk> {
    state
        .artifact_read(&session_id, &hash, offset, length)
        .await
}
#[tauri::command]
#[specta::specta]
pub async fn artifact_save(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    hash: String,
    path: String,
) -> AppResult<()> {
    state
        .artifact_save(&session_id, &hash, &PathBuf::from(path))
        .await
}
