//! Official MCP SDK adapter. Remote calls are never described as locally sandboxed.
//! The host creates stdio processes and enforces the selected execution policy.
use crate::manifest::{fail, identifier};
use harness_contracts::{AppResult, ToolDefinition, ToolResult};
use rmcp::{
    ServiceExt,
    service::{RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
/// Select one process-wide provider even when dependency feature unification enables two.
pub fn init_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "camelCase")]
pub enum ServerConfig {
    Stdio {
        id: String,
        command: String,
        args: Vec<String>,
    },
    StreamableHttp {
        id: String,
        url: String,
    },
}
impl ServerConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::Stdio { id, .. } | Self::StreamableHttp { id, .. } => id,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ConnectionOptions {
    pub startup_timeout: Duration,
    pub tool_timeout: Duration,
    pub enabled_tools: Option<BTreeSet<String>>,
    pub disabled_tools: BTreeSet<String>,
}
impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(60),
            enabled_tools: None,
            disabled_tools: BTreeSet::new(),
        }
    }
}
fn short_hash(value: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, value.as_bytes())
        .simple()
        .to_string()[..16]
        .to_owned()
}
fn visible_tool_name(id: &str, tool: &str) -> String {
    let server = if id.len() > 40 {
        format!("{}-{}", &id[..20], short_hash(id))
    } else {
        id.to_owned()
    };
    let prefix = format!("mcp__{server}__");
    if prefix.len() + tool.len() <= 64
        && tool
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return format!("{prefix}{tool}");
    }
    let sanitized: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let available = 64usize.saturating_sub(prefix.len() + 17);
    format!(
        "{prefix}{}-{}",
        &sanitized[..sanitized.len().min(available)],
        short_hash(tool)
    )
}
struct Connection {
    service: RunningService<RoleClient, ()>,
    tools: Vec<ToolDefinition>,
    tool_names: BTreeMap<String, String>,
    generation: CancellationToken,
    tool_timeout: Duration,
}
#[derive(Default)]
pub struct McpManager {
    servers: RwLock<BTreeMap<String, Arc<Connection>>>,
}
impl McpManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn connect_http(&self, config: &ServerConfig, authorized: bool) -> AppResult<()> {
        self.connect_http_with_headers(config, &BTreeMap::new(), authorized)
            .await
    }
    pub async fn connect_http_with_headers(
        &self,
        config: &ServerConfig,
        headers: &BTreeMap<String, String>,
        authorized: bool,
    ) -> AppResult<()> {
        self.connect_http_with_options(config, headers, authorized, &ConnectionOptions::default())
            .await
    }
    pub async fn connect_http_with_options(
        &self,
        config: &ServerConfig,
        headers: &BTreeMap<String, String>,
        authorized: bool,
        settings: &ConnectionOptions,
    ) -> AppResult<()> {
        init_tls();
        if !authorized {
            return Err(fail("explicit MCP connection authorization required"));
        }
        let ServerConfig::StreamableHttp { id, url } = config else {
            return Err(fail("HTTP transport required"));
        };
        if !identifier(id) {
            return Err(fail("invalid MCP server id"));
        }
        let parsed = reqwest::Url::parse(url).map_err(fail)?;
        if !["http", "https"].contains(&parsed.scheme())
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(fail("invalid MCP HTTP URL"));
        }
        let mut resolved_headers = std::collections::HashMap::new();
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| fail("invalid MCP header name"))?;
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "content-type"
                    | "accept"
                    | "mcp-session-id"
                    | "mcp-protocol-version"
            ) {
                return Err(fail("reserved MCP header"));
            }
            let mut value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| fail("invalid MCP header value"))?;
            value.set_sensitive(true);
            resolved_headers.insert(name, value);
        }
        let options = StreamableHttpClientTransportConfig::with_uri(url.as_str())
            .reinit_on_expired_session(false)
            .custom_headers(resolved_headers);
        let transport = StreamableHttpClientTransport::from_config(options);
        let deadline = tokio::time::Instant::now() + settings.startup_timeout;
        let service = tokio::time::timeout_at(deadline, ().serve(transport))
            .await
            .map_err(fail)?
            .map_err(fail)?;
        self.register(id, service, settings, deadline).await
    }
    /// The host must authorize process creation and apply the selected execution policy.
    /// This adapter accepts pipes and never starts the configured command.
    pub async fn connect_sandboxed_stdio<R, W>(
        &self,
        config: &ServerConfig,
        read: R,
        write: W,
        authorized: bool,
    ) -> AppResult<()>
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
        W: tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        self.connect_stdio_with_options(
            config,
            read,
            write,
            authorized,
            &ConnectionOptions::default(),
        )
        .await
    }
    pub async fn connect_stdio_with_options<R, W>(
        &self,
        config: &ServerConfig,
        read: R,
        write: W,
        authorized: bool,
        settings: &ConnectionOptions,
    ) -> AppResult<()>
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
        W: tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        if !authorized {
            return Err(fail("explicit MCP process authorization required"));
        }
        let ServerConfig::Stdio { id, .. } = config else {
            return Err(fail("stdio config required"));
        };
        if !identifier(id) {
            return Err(fail("invalid MCP server id"));
        }
        let deadline = tokio::time::Instant::now() + settings.startup_timeout;
        let service = tokio::time::timeout_at(deadline, ().serve((read, write)))
            .await
            .map_err(fail)?
            .map_err(fail)?;
        self.register(id, service, settings, deadline).await
    }
    async fn register(
        &self,
        id: &str,
        service: RunningService<RoleClient, ()>,
        settings: &ConnectionOptions,
        deadline: tokio::time::Instant,
    ) -> AppResult<()> {
        let list = tokio::time::timeout_at(deadline, service.peer().list_all_tools())
            .await
            .map_err(fail)?
            .map_err(fail)?;
        let mut tools = Vec::new();
        let mut tool_names = BTreeMap::new();
        for t in list {
            let name = t.name.to_string();
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            {
                return Err(fail("MCP tool name outside supported stable-name alphabet"));
            }
            if settings
                .enabled_tools
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(&name))
                || settings.disabled_tools.contains(&name)
            {
                continue;
            }
            let visible_name = visible_tool_name(id, &name);
            if tool_names
                .insert(visible_name.clone(), name.clone())
                .is_some()
            {
                return Err(fail("MCP tool name collision"));
            }
            tools.push(ToolDefinition {
                name: visible_name,
                description: format!(
                    "{} / {}: {}",
                    id,
                    name,
                    t.description.as_deref().unwrap_or("")
                ),
                parameters: serde_json::to_value(t.input_schema).map_err(fail)?,
            });
        }
        let conn = Arc::new(Connection {
            service,
            tools,
            tool_names,
            generation: CancellationToken::new(),
            tool_timeout: settings.tool_timeout,
        });
        let mut servers = self.servers.write().await;
        if servers.iter().any(|(existing_id, existing)| {
            existing_id != id
                && conn
                    .tool_names
                    .keys()
                    .any(|name| existing.tool_names.contains_key(name))
        }) {
            return Err(fail("MCP tool name collision"));
        }
        let old = servers.insert(id.into(), conn);
        if let Some(old) = old {
            old.generation.cancel();
        }
        Ok(())
    }
    pub async fn disconnect(&self, id: &str) -> AppResult<()> {
        let old = self
            .servers
            .write()
            .await
            .remove(id)
            .ok_or_else(|| fail("MCP server missing"))?;
        old.generation.cancel();
        Ok(())
    }
    pub async fn is_connected(&self, id: &str) -> bool {
        self.servers.read().await.get(id).is_some_and(|connection| {
            !connection.service.is_closed() && !connection.service.peer().is_transport_closed()
        })
    }
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.servers
            .read()
            .await
            .values()
            .filter(|c| !c.service.is_closed() && !c.service.peer().is_transport_closed())
            .flat_map(|c| c.tools.clone())
            .collect()
    }
    pub async fn execute_tool(
        &self,
        name: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        if cancel.is_cancelled() {
            return Err(fail("MCP call cancelled before dispatch"));
        }
        let (c, tool) = self
            .servers
            .read()
            .await
            .values()
            .find_map(|connection| {
                connection
                    .tool_names
                    .get(name)
                    .map(|original| (connection.clone(), original.clone()))
            })
            .ok_or_else(|| fail("MCP tool not registered or disconnected"))?;
        let arguments = args
            .as_object()
            .cloned()
            .ok_or_else(|| fail("MCP arguments must be object"))?;
        let request = rmcp::model::CallToolRequestParams::new(tool).with_arguments(arguments);
        let result = tokio::select! {biased; _ = cancel.cancelled()=>return Err(fail("MCP cancelled; remote side effects may have happened")),_ = c.generation.cancelled()=>return Err(fail("MCP connection replaced/disconnected")),result=tokio::time::timeout(c.tool_timeout,c.service.peer().call_tool_once(request))=>result.map_err(fail)?.map_err(fail)?};
        let rmcp::model::CallToolResponse::Complete(result) = result else {
            return Err(fail(
                "MCP input/task continuation requires explicit host handling; call was not retried",
            ));
        };
        let structured_content = result.structured_content.clone();
        let content = if result.content.is_empty() {
            match result.structured_content {
                Some(value) => serde_json::to_string(&value).map_err(fail)?,
                None => "[]".to_owned(),
            }
        } else {
            let blocks: Vec<_> = result
                .content
                .into_iter()
                .map(|block| match block {
                    rmcp::model::ContentBlock::Image(_) => {
                        rmcp::model::ContentBlock::text("[MCP 图片暂不支持直接传给模型]")
                    }
                    rmcp::model::ContentBlock::Audio(_) => {
                        rmcp::model::ContentBlock::text("[MCP 音频暂不支持直接传给模型]")
                    }
                    rmcp::model::ContentBlock::Resource(resource)
                        if matches!(
                            resource.resource,
                            rmcp::model::ResourceContents::BlobResourceContents { .. }
                        ) =>
                    {
                        rmcp::model::ContentBlock::text("[MCP 二进制资源暂不支持直接传给模型]")
                    }
                    other => other,
                })
                .collect();
            serde_json::to_string(&blocks).map_err(fail)?
        };
        Ok(ToolResult {
            structured_content,
            content,
            is_error: result.is_error.unwrap_or(false),
            diff: None,
            escalated: false,
        })
    }
    pub async fn list_resources(&self, id: &str) -> AppResult<Value> {
        let c = self
            .servers
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| fail("MCP disconnected"))?;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            c.service.peer().list_all_resources(),
        )
        .await
        .map_err(fail)?
        .map_err(fail)?;
        serde_json::to_value(result).map_err(fail)
    }
    pub async fn read_resource(&self, id: &str, uri: &str) -> AppResult<Value> {
        let c = self
            .servers
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| fail("MCP disconnected"))?;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            c.service
                .peer()
                .read_resource(rmcp::model::ReadResourceRequestParams::new(uri)),
        )
        .await
        .map_err(fail)?
        .map_err(fail)?;
        serde_json::to_value(result).map_err(fail)
    }
}
