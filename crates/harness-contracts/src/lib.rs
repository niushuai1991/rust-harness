//! Shared wire vocabulary. Persistent event cursors are decimal strings to avoid JS precision loss.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
/// A named recursive JSON definition prevents infinite inlining during TS export.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    Number(f64),
    Text(String),
    Array(Vec<JsonValue>),
    Object(std::collections::HashMap<String, JsonValue>),
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct JsonPayload(#[specta(type = JsonValue)] pub Value);

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}
impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AppError {}
pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: String,
    pub source_path: String,
    pub replica_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub kind: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub source_hash: Option<String>,
    pub conflict: bool,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub seq: String,
    pub kind: String,
    #[specta(type = JsonValue)]
    pub data: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LiveEvent {
    Durable {
        event: SessionEvent,
    },
    Delta {
        session_id: String,
        request_id: String,
        index: u32,
        text: String,
        reasoning: bool,
    },
    Status {
        session_id: String,
        status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[specta(type = JsonValue)]
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[specta(type = JsonValue)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    #[specta(type = JsonValue)]
    pub arguments: Value,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ApiMode {
    #[default]
    ChatCompletions,
    Responses,
    Anthropic,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEndpoint {
    pub api_mode: ApiMode,
    pub base_url: String,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProviderCompatibility {
    #[default]
    Standard,
    DeepSeek,
    BigModel,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub endpoints: Vec<ProtocolEndpoint>,
    pub default_model: String,
    pub credential_ref: String,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub api_mode: ApiMode,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub credential_ref: String,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub enabled: bool,
    pub permissions: Vec<String>,
    pub status: String,
    pub description: String,
}

/// Host-managed, content-addressed attachment metadata. Never includes the imported host path.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub hash: String,
    pub name: String,
    pub media_type: String,
    pub size: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChunk {
    pub encoding: String,
    pub content: String,
    pub next_offset: Option<u32>,
    pub total_size: u32,
}
