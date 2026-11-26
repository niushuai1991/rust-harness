use std::{collections::BTreeMap, sync::Arc};

use harness_contracts::{AppError, AppResult};
use harness_storage::Store;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

const CONFIG_KEY: &str = "mcp.config.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "workspaceId", rename_all = "camelCase")]
pub enum McpScope {
    Global,
    Project(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, SecretReference>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, SecretReference>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "source",
    content = "name",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum SecretReference {
    Environment(String),
    Credential(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    pub id: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub transport: McpTransport,
    #[serde(default = "startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "tool_timeout")]
    pub tool_timeout_secs: u64,
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}
fn enabled() -> bool {
    true
}
fn startup_timeout() -> u64 {
    30
}
fn tool_timeout() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveMcpConfig {
    pub scope: McpScope,
    pub config: McpServerConfig,
    pub trusted: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredConfigs {
    global: BTreeMap<String, McpServerConfig>,
    projects: BTreeMap<String, BTreeMap<String, ProjectConfig>>,
}
#[derive(Debug, Serialize, Deserialize)]
struct ProjectConfig {
    config: McpServerConfig,
    trusted: bool,
}

#[derive(Clone)]
pub struct McpConfigService {
    store: Store,
    gate: Arc<Mutex<()>>,
}
impl McpConfigService {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            gate: Arc::new(Mutex::new(())),
        }
    }
    async fn read(&self) -> AppResult<StoredConfigs> {
        match self.store.get(CONFIG_KEY).await? {
            Some(value) => serde_json::from_value(value).map_err(|_| invalid("服务器配置损坏")),
            None => Ok(StoredConfigs::default()),
        }
    }
    async fn write(&self, configs: &StoredConfigs) -> AppResult<()> {
        let value = serde_json::to_value(configs).map_err(|_| invalid("服务器配置无法保存"))?;
        self.store.set(CONFIG_KEY, &value).await
    }
    pub async fn save(&self, scope: McpScope, config: McpServerConfig) -> AppResult<()> {
        self.save_inner(scope, config, false).await
    }
    pub async fn create(&self, scope: McpScope, config: McpServerConfig) -> AppResult<()> {
        self.save_inner(scope, config, true).await
    }
    async fn save_inner(
        &self,
        scope: McpScope,
        config: McpServerConfig,
        create_only: bool,
    ) -> AppResult<()> {
        validate(&scope, &config)?;
        let _guard = self.gate.lock().await;
        let mut configs = self.read().await?;
        let exists = match &scope {
            McpScope::Global => configs.global.contains_key(&config.id),
            McpScope::Project(workspace) => configs
                .projects
                .get(workspace)
                .is_some_and(|entries| entries.contains_key(&config.id)),
        };
        if create_only && exists {
            return Err(invalid("该范围内的服务器标识已存在"));
        }
        match scope {
            McpScope::Global => {
                configs.global.insert(config.id.clone(), config);
            }
            McpScope::Project(workspace) => {
                let entries = configs.projects.entry(workspace).or_default();
                let trusted = entries
                    .get(&config.id)
                    .is_some_and(|old| old.trusted && old.config == config);
                entries.insert(config.id.clone(), ProjectConfig { config, trusted });
            }
        }
        self.write(&configs).await
    }
    pub async fn remove(&self, scope: McpScope, id: &str) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        let mut configs = self.read().await?;
        let removed = match scope {
            McpScope::Global => configs.global.remove(id).is_some(),
            McpScope::Project(workspace) => configs
                .projects
                .get_mut(&workspace)
                .and_then(|entries| entries.remove(id))
                .is_some(),
        };
        if !removed {
            return Err(invalid("服务器配置不存在"));
        }
        self.write(&configs).await
    }
    pub async fn trust_exact(&self, scope: McpScope, expected: &McpServerConfig) -> AppResult<()> {
        let McpScope::Project(workspace) = scope else {
            return Err(invalid("只有项目配置需要信任"));
        };
        let _guard = self.gate.lock().await;
        let mut configs = self.read().await?;
        let entry = configs
            .projects
            .get_mut(&workspace)
            .and_then(|servers| servers.get_mut(&expected.id))
            .ok_or_else(|| invalid("服务器配置不存在"))?;
        if &entry.config != expected {
            return Err(invalid("配置已发生变化，请重新确认"));
        }
        entry.trusted = true;
        self.write(&configs).await
    }
    pub async fn effective(&self, workspace: Option<&str>) -> AppResult<Vec<EffectiveMcpConfig>> {
        let _guard = self.gate.lock().await;
        let configs = self.read().await?;
        let mut result: BTreeMap<String, EffectiveMcpConfig> = configs
            .global
            .into_iter()
            .map(|(id, config)| {
                (
                    id,
                    EffectiveMcpConfig {
                        scope: McpScope::Global,
                        config,
                        trusted: true,
                    },
                )
            })
            .collect();
        if let Some(workspace) = workspace.filter(|w| !w.is_empty())
            && let Some(project) = configs.projects.get(workspace)
        {
            for (id, entry) in project {
                result.insert(
                    id.clone(),
                    EffectiveMcpConfig {
                        scope: McpScope::Project(workspace.to_owned()),
                        config: entry.config.clone(),
                        trusted: entry.trusted,
                    },
                );
            }
        }
        Ok(result.into_values().collect())
    }
}
fn invalid(message: &str) -> AppError {
    AppError::new("MCP_CONFIG", message)
}
fn validate(scope: &McpScope, config: &McpServerConfig) -> AppResult<()> {
    if config.id.is_empty()
        || config.id.len() > 100
        || !config
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid("服务器标识无效"));
    }
    if matches!(scope, McpScope::Project(w) if w.is_empty()) {
        return Err(invalid("项目标识不能为空"));
    }
    if !(1..=3600).contains(&config.startup_timeout_secs)
        || !(1..=3600).contains(&config.tool_timeout_secs)
    {
        return Err(invalid("超时时间必须在 1–3600 秒之间"));
    }
    match &config.transport {
        McpTransport::Stdio {
            command,
            args,
            cwd,
            env,
        } => {
            if command.trim().is_empty()
                || command.contains('\0')
                || args.iter().any(|a| a.contains('\0'))
                || cwd.as_ref().is_some_and(|c| c.contains('\0'))
            {
                return Err(invalid("本地服务器命令无效"));
            }
            if let Some(cwd) = cwd.as_deref().filter(|value| !value.is_empty()) {
                let path = std::path::Path::new(cwd);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|part| !matches!(part, std::path::Component::Normal(_)))
                {
                    return Err(invalid("MCP 工作目录必须是执行根内的相对路径"));
                }
            }
            validate_refs(env)?;
            if env.keys().any(|name| name.contains('=')) {
                return Err(invalid("环境变量名称无效"));
            }
        }
        McpTransport::StreamableHttp { url, headers } => {
            let parsed = reqwest::Url::parse(url).map_err(|_| invalid("服务器地址无效"))?;
            if !matches!(parsed.scheme(), "http" | "https")
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(invalid("服务器地址无效"));
            }
            validate_refs(headers)?;
            for name in headers.keys() {
                let parsed = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| invalid("请求头名称无效"))?;
                if matches!(
                    parsed.as_str(),
                    "host"
                        | "content-length"
                        | "content-type"
                        | "accept"
                        | "mcp-session-id"
                        | "mcp-protocol-version"
                ) {
                    return Err(invalid("不能覆盖协议请求头"));
                }
            }
        }
    }
    Ok(())
}
fn validate_refs(refs: &BTreeMap<String, SecretReference>) -> AppResult<()> {
    for (key, reference) in refs {
        let name = match reference {
            SecretReference::Environment(name) | SecretReference::Credential(name) => name,
        };
        if key.trim().is_empty()
            || key.contains(['\0', '\r', '\n'])
            || name.trim().is_empty()
            || name.contains(['\0', '\r', '\n'])
        {
            return Err(invalid("凭据引用无效"));
        }
    }
    Ok(())
}

pub(crate) fn resolve_references(
    references: &BTreeMap<String, SecretReference>,
) -> AppResult<BTreeMap<String, String>> {
    references
        .iter()
        .map(|(name, reference)| {
            let value = match reference {
                SecretReference::Environment(key) => std::env::var(key).ok(),
                SecretReference::Credential(key) => harness_platform::get_secret(key)?,
            }
            .ok_or_else(|| AppError::new("MCP_CREDENTIAL", "服务器所需的凭据引用不可用"))?;
            Ok((name.clone(), value))
        })
        .collect()
}
