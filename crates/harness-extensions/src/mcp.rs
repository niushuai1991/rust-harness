//! Official MCP SDK adapter. Remote calls are never described as locally sandboxed.
//! Stdio process creation belongs to the platform sandbox; this module accepts its pipes.
use crate::manifest::{fail, identifier};
use harness_contracts::{AppResult, ToolDefinition, ToolResult};
use rmcp::{
    ServiceExt,
    service::{RoleClient, RunningService},
    transport::StreamableHttpClientTransport,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
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
struct Connection {
    service: RunningService<RoleClient, ()>,
    tools: Vec<ToolDefinition>,
    generation: CancellationToken,
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
        let transport = StreamableHttpClientTransport::from_uri(url.as_str());
        let service = tokio::time::timeout(Duration::from_secs(30), ().serve(transport))
            .await
            .map_err(fail)?
            .map_err(fail)?;
        self.register(id, service).await
    }
    /// The caller must obtain user approval for ServerConfig and create both pipes via the platform sandbox.
    /// This does not execute the supplied command and cannot be used to bypass the shell policy.
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
        if !authorized {
            return Err(fail("explicit MCP process authorization required"));
        }
        let ServerConfig::Stdio { id, .. } = config else {
            return Err(fail("stdio config required"));
        };
        if !identifier(id) {
            return Err(fail("invalid MCP server id"));
        }
        let service = tokio::time::timeout(Duration::from_secs(30), ().serve((read, write)))
            .await
            .map_err(fail)?
            .map_err(fail)?;
        self.register(id, service).await
    }
    async fn register(&self, id: &str, service: RunningService<RoleClient, ()>) -> AppResult<()> {
        let list = tokio::time::timeout(Duration::from_secs(30), service.peer().list_all_tools())
            .await
            .map_err(fail)?
            .map_err(fail)?;
        let mut tools = Vec::new();
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
            tools.push(ToolDefinition {
                name: format!("mcp__{id}__{name}"),
                description: t.description.as_deref().unwrap_or("").to_owned(),
                parameters: serde_json::to_value(t.input_schema).map_err(fail)?,
            });
        }
        let conn = Arc::new(Connection {
            service,
            tools,
            generation: CancellationToken::new(),
        });
        let old = self.servers.write().await.insert(id.into(), conn);
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
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.servers
            .read()
            .await
            .values()
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
        let (id, tool) = name
            .strip_prefix("mcp__")
            .and_then(|n| n.split_once("__"))
            .ok_or_else(|| fail("invalid MCP name"))?;
        let c = self
            .servers
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| fail("MCP disconnected"))?;
        if !c.tools.iter().any(|t| t.name == name) {
            return Err(fail("MCP tool not registered"));
        }
        let arguments = args
            .as_object()
            .cloned()
            .ok_or_else(|| fail("MCP arguments must be object"))?;
        let request =
            rmcp::model::CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
        let result = tokio::select! {biased; _ = cancel.cancelled()=>return Err(fail("MCP cancelled; remote side effects may have happened")),_ = c.generation.cancelled()=>return Err(fail("MCP connection replaced/disconnected")),result=tokio::time::timeout(Duration::from_secs(60),c.service.peer().call_tool_once(request))=>result.map_err(fail)?.map_err(fail)?};
        let rmcp::model::CallToolResponse::Complete(result) = result else {
            return Err(fail(
                "MCP input/task continuation requires explicit host handling; call was not retried",
            ));
        };
        Ok(ToolResult {
            content: serde_json::to_string(&result.content).map_err(fail)?,
            is_error: result.is_error.unwrap_or(false),
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
