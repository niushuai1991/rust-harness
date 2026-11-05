use harness_extensions::mcp::{McpManager, ServerConfig};
use rmcp::{
    ServerHandler, ServiceExt,
    model::*,
    service::{RequestContext, RoleServer},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
struct Fixture;
impl ServerHandler for Fixture {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        let mut info = rmcp::model::ServerConfig::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        info
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let tools = vec![
            serde_json::from_value(
                json!({"name":"echo","description":"fixture","inputSchema":{"type":"object"}}),
            )
            .unwrap(),
        ];
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text("actual MCP response")]).into())
    }
}
#[tokio::test]
async fn official_sdk_stdio_handshake_register_call_resources_and_disconnect() {
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = Fixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "fixture".into(),
        command: "approved sandbox process".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    assert_eq!(
        manager.tool_definitions().await[0].name,
        "mcp__fixture__echo"
    );
    let result = manager
        .execute_tool("mcp__fixture__echo", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(result.content.contains("actual MCP response"));
    assert!(!result.is_error);
    assert_eq!(manager.list_resources("fixture").await.unwrap(), json!([]));
    manager.disconnect("fixture").await.unwrap();
    assert!(manager.tool_definitions().await.is_empty());
    assert!(
        manager
            .execute_tool("mcp__fixture__echo", json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    server.abort();
}
#[tokio::test]
async fn unauthorized_http_never_connects() {
    let manager = McpManager::new();
    let config = ServerConfig::StreamableHttp {
        id: "blocked".into(),
        url: "http://127.0.0.1:1/mcp".into(),
    };
    assert!(manager.connect_http(&config, false).await.is_err());
}
#[tokio::test]
async fn official_sdk_streamable_http_handshake_and_tools() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut header = Vec::new();
                loop {
                    let b = socket.read_u8().await.unwrap();
                    header.push(b);
                    assert!(header.len() < 16384);
                    if header.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8(header).unwrap();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if !head.starts_with("POST ") {
                    socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    return;
                }
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let Some(id) = request.get("id") else {
                    socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    return;
                };
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => {
                        json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"http-fixture","version":"1"}})
                    }
                    "tools/list" => {
                        json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                    }
                    "tools/call" => json!({"content":[{"type":"text","text":"http actual"}]}),
                    method => panic!("unexpected fixture method {method}"),
                };
                let response = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    let m = McpManager::new();
    let config = ServerConfig::StreamableHttp {
        id: "http".into(),
        url: format!("http://{address}/mcp"),
    };
    m.connect_http(&config, true).await.unwrap();
    let result = m
        .execute_tool("mcp__http__echo", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(result.content.contains("http actual"));
    m.disconnect("http").await.unwrap();
    server.abort();
}
#[derive(Clone)]
struct SlowFixture {
    started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl ServerHandler for SlowFixture {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        Fixture.get_info()
    }
    async fn list_tools(
        &self,
        p: Option<PaginatedRequestParams>,
        c: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Fixture.list_tools(p, c).await
    }
    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        c: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        c.ct.cancelled().await;
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(CallToolResult::success(vec![]).into())
    }
}
#[tokio::test]
async fn cancellation_notifies_server_and_reconnect_never_replays_tool() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let started = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let fixture = SlowFixture {
        started: started.clone(),
        cancelled: cancelled.clone(),
    };
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = fixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = Arc::new(McpManager::new());
    let config = ServerConfig::Stdio {
        id: "cancel".into(),
        command: "approved".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let child = cancel.clone();
    let runner = manager.clone();
    let call = tokio::spawn(async move {
        runner
            .execute_tool("mcp__cancel__echo", json!({}), child)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(call.await.unwrap().is_err());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 1);
    let (client, newserver) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(newserver);
    let replacement = tokio::spawn(async move {
        let service = Fixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 1);
    manager.disconnect("cancel").await.unwrap();
    server.abort();
    replacement.abort();
}
