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

/// 单文件 unified diff 视图（桌面端右侧面板懒加载）；unified 为 None 表示二进制或超限。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileDiff {
    pub path: String,
    pub unified: Option<String>,
}

/// 工作副本文件的有界预览（桌面端右侧面板）。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePreview {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub binary: bool,
    #[specta(type = f64)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    /// 子代理归属的父会话；由运行时从 subagent.created 事件推导，存储层恒为空。
    pub parent_id: Option<String>,
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

/// 供应商认证方式：静态 API key 或 ChatGPT 账号 OAuth（token 由登录流程动态维护）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAuth {
    #[default]
    ApiKey,
    ChatGptOAuth,
}
/// 已知推理档位，语义强度从低到高；efforts 为空表示该模型不提供 effort 选择。
pub const KNOWN_EFFORTS: [&str; 6] = ["disabled", "minimal", "low", "medium", "high", "max"];

/// 已知审批策略档位，与 CLI/ACP 的三档策略一一对应。
pub const KNOWN_APPROVALS: [&str; 3] = ["ask", "auto", "never"];

/// 内置供应商随程序分发的只读模型条目；启用状态由个人层的 `disabled_models` 覆盖。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinModel {
    pub id: String,
    pub name: String,
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    #[serde(default)]
    pub auth: ProviderAuth,
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub endpoints: Vec<ProtocolEndpoint>,
    pub default_model: String,
    pub credential_ref: String,
    pub proxy: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub builtin_models: Vec<BuiltinModel>,
    #[serde(default)]
    pub disabled_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    #[serde(default)]
    pub auth: ProviderAuth,
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
    /// 支持的推理档位；为空表示不支持，UI 隐藏 effort 选择。
    #[serde(default)]
    pub efforts: Vec<String>,
    /// 当前默认档位；None 表示不干预、跟随供应商默认行为。
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 由内置目录合成的条目；不可编辑或删除，只能启用/禁用。
    #[serde(default)]
    pub builtin: bool,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            compatibility: ProviderCompatibility::default(),
            auth: ProviderAuth::default(),
            provider_id: String::new(),
            api_mode: ApiMode::default(),
            max_output_tokens: None,
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
            credential_ref: String::new(),
            proxy: None,
            efforts: Vec::new(),
            effort: None,
            enabled: true,
            builtin: false,
        }
    }
}

/// 供应商剩余额度的卡片类别；与智谱 quota/limit 接口的 type/unit 匹配规则对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UsageCardKind {
    FiveHour,
    Weekly,
    Tools,
}

/// 单张剩余额度卡；`remaining_percent` 为剩余百分比（接口的已用口径已反转），`reset_at` 为毫秒时间戳。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageCard {
    pub kind: UsageCardKind,
    pub remaining_percent: Option<f64>,
    /// 毫秒时间戳远小于 2^53，specta 以 f64 导出 number，绕开 BigInt 禁令。
    #[specta(type = f64)]
    pub reset_at: Option<i64>,
}

/// 供应商套餐与剩余额度快照；即查即返，不落库。
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageSnapshot {
    /// 套餐显示名（如 GLM Coding Pro）；由接口 level 字段归一化而来。
    pub plan_name: Option<String>,
    pub cards: Vec<ProviderUsageCard>,
}

/// ChatGPT 登录流程状态机（桌面端轮询）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChatGptLoginStatus {
    /// 本窗口未发起过登录。
    #[default]
    Idle,
    /// 等待浏览器回调。
    Pending,
    /// 登录成功且凭据已保存。
    Completed,
    /// 登录失败（error 携带原因）。
    Failed,
    /// 用户取消。
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptLoginState {
    pub status: ChatGptLoginStatus,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub error: Option<String>,
}

/// ChatGPT 账号登录信息查询；不含任何 token。
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAccountInfo {
    pub logged_in: bool,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    /// access token 到期时间（Unix 秒，远小于 2^53）。
    #[specta(type = f64)]
    pub expires_at: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// 桌面端跨重启保留的上次模型选择；effort 为 None 表示跟随模型默认。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSelection {
    pub model_id: String,
    pub effort: Option<String>,
}

/// 用户自定义预设：命名的会话能力配置包（模型/effort/审批策略），可选字段缺省表示该项不覆盖。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresetConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub approval: Option<String>,
}

/// 预设列表行：配置附运行时计算的 broken 标记（不落库），可读但不可直接构建模型。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    #[serde(flatten)]
    pub preset: PresetConfig,
    /// 预设不可用的原因（模型缺失/停用、effort 不支持等）；None 表示可用。
    pub broken: Option<String>,
}

/// 技能目录行：SKILL.md 扫描结果附禁用标记（docs/specs/skills-plan.md）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// 项目级为相对工作副本的路径，用户级为绝对路径；统一正斜杠。
    pub path: String,
    pub scope: SkillScope,
    pub enabled: bool,
    /// 技能不可用的原因（name 非法等）；None 表示可用。
    #[serde(default)]
    pub broken_reason: Option<String>,
}

/// 技能来源层：项目级（工作副本）与用户级（主目录 `~/.agents/skills/`）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillScope {
    Project,
    User,
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

/// One configured language server: the command runs inside the platform sandbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LspServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Server-specific `initializationOptions`; the harness pushes file contents
    /// itself, so servers needing it may disable their internal file watchers here.
    #[serde(default)]
    #[specta(type = JsonValue)]
    pub initialization_options: Value,
}

impl LspServerConfig {
    pub fn default_servers() -> Vec<Self> {
        vec![Self {
            id: "rust-analyzer".into(),
            command: "rust-analyzer".into(),
            args: vec![],
            extensions: vec!["rs".into()],
            initialization_options: Value::Null,
        }]
    }
}

/// Runtime state of one language server connection, for host status queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LspStatusEntry {
    pub workspace_id: String,
    pub server_id: String,
    pub running: bool,
    pub documents: u32,
    pub diagnostics: u32,
}

/// Latest todo/plan/goal entry of one session for the desktop overview panel.
/// Derived view over the event log; the event stream stays the source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanStateOverview {
    pub session_id: String,
    pub session_title: String,
    pub workspace_id: String,
    pub kind: String,
    #[specta(type = JsonValue)]
    pub data: Value,
    pub updated_at: String,
}

#[cfg(test)]
mod lsp_config_tests {
    use super::*;

    #[test]
    fn default_servers_map_rust_to_rust_analyzer() {
        let servers = LspServerConfig::default_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "rust-analyzer");
        assert_eq!(servers[0].command, "rust-analyzer");
        assert_eq!(servers[0].extensions, vec!["rs"]);
        assert!(servers[0].args.is_empty());
    }

    #[test]
    fn config_roundtrips_with_missing_optional_fields() {
        let config: LspServerConfig = serde_json::from_str(
            r#"{"id":"pyright","command":"pyright-langserver","extensions":["py"]}"#,
        )
        .unwrap();
        assert_eq!(config.args, Vec::<String>::new());
        assert_eq!(config.extensions, vec!["py"]);
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    /// 预设可选字段序列化缺省；camelCase 键名与桌面绑定一致。
    #[test]
    fn preset_roundtrips_with_optional_fields() {
        let preset = PresetConfig {
            id: "refactor".into(),
            name: "批量重构".into(),
            description: Some("自动审批跑批量任务".into()),
            model_id: Some("builtin:bigmodel:glm-5.3".into()),
            effort: Some("high".into()),
            approval: Some("auto".into()),
        };
        let json = serde_json::to_string(&preset).unwrap();
        assert!(json.contains("\"modelId\""));
        let back: PresetConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "refactor");
        assert_eq!(back.model_id.as_deref(), Some("builtin:bigmodel:glm-5.3"));
        assert_eq!(back.effort.as_deref(), Some("high"));
        assert_eq!(back.approval.as_deref(), Some("auto"));
    }

    /// 旧存储/手写 JSON 允许只给 id，其余字段全部缺省。
    #[test]
    fn preset_minimal_json_defaults() {
        let preset: PresetConfig = serde_json::from_str(r#"{"id":"p1","name":"p1"}"#).unwrap();
        assert_eq!(preset.description, None);
        assert_eq!(preset.model_id, None);
        assert_eq!(preset.effort, None);
        assert_eq!(preset.approval, None);
    }

    /// 审批档位集合与 CLI/ACP 的三档策略一一对应。
    #[test]
    fn known_approvals_cover_policy_set() {
        assert_eq!(KNOWN_APPROVALS, ["ask", "auto", "never"]);
    }
}

#[cfg(test)]
mod model_settings_tests {
    use super::*;

    /// 升级兼容：v2 存储中的旧供应商/模型 JSON 不含新增字段，必须原样读入并取默认值。
    #[test]
    fn legacy_provider_and_model_json_defaults() {
        let provider: ProviderInfo = serde_json::from_str(
            r#"{"id":"deepseek","name":"DeepSeek","builtin":true,
                "endpoints":[{"apiMode":"anthropic","baseUrl":"https://api.deepseek.com/anthropic"}],
                "defaultModel":"deepseek-flash","credentialRef":"provider/deepseek","proxy":null}"#,
        )
        .unwrap();
        assert!(provider.enabled, "旧数据供应商默认启用");
        assert!(provider.builtin_models.is_empty());
        assert!(provider.disabled_models.is_empty());

        let model: ModelConfig = serde_json::from_str(
            r#"{"id":"m1","name":"测试","baseUrl":"https://api.deepseek.com/anthropic",
                "model":"deepseek-flash","credentialRef":"provider/deepseek","proxy":null,
                "providerId":"deepseek","compatibility":"deepSeek","apiMode":"anthropic",
                "maxOutputTokens":null}"#,
        )
        .unwrap();
        assert!(model.enabled, "旧数据模型默认启用");
        assert!(!model.builtin);
        assert_eq!(model.effort, None);
        assert!(model.efforts.is_empty());
    }

    #[test]
    fn effort_lists_roundtrip() {
        let model = ModelConfig {
            efforts: vec!["low".into(), "high".into()],
            effort: Some("high".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        let back: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.efforts, model.efforts);
        assert_eq!(back.effort, model.effort);
    }

    #[test]
    fn known_efforts_are_ordered_low_to_high() {
        assert_eq!(
            KNOWN_EFFORTS,
            ["disabled", "minimal", "low", "medium", "high", "max"]
        );
    }

    /// 新增 auth 字段对旧 JSON 缺省为 ApiKey；ChatGptOAuth 以 camelCase 序列化。
    #[test]
    fn provider_auth_defaults_and_roundtrip() {
        let provider: ProviderInfo = serde_json::from_str(
            r#"{"id":"chatgpt","name":"ChatGPT","builtin":true,
                "endpoints":[{"apiMode":"responses","baseUrl":"https://chatgpt.com/backend-api/codex"}],
                "defaultModel":"gpt-5.5","credentialRef":"provider/chatgpt","proxy":null,
                "auth":"chatGptOAuth"}"#,
        )
        .unwrap();
        assert_eq!(provider.auth, ProviderAuth::ChatGptOAuth);

        let legacy: ProviderInfo = serde_json::from_str(
            r#"{"id":"deepseek","name":"DeepSeek","builtin":true,
                "endpoints":[],"defaultModel":"deepseek-flash",
                "credentialRef":"provider/deepseek","proxy":null}"#,
        )
        .unwrap();
        assert_eq!(legacy.auth, ProviderAuth::ApiKey);

        let model = ModelConfig {
            auth: ProviderAuth::ChatGptOAuth,
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("\"auth\":\"chatGptOAuth\""));
    }
}
