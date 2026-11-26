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
#[derive(Clone)]
struct StructuredFixture;
impl ServerHandler for StructuredFixture {
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
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let mut result = CallToolResult::success(vec![]);
        result.structured_content = Some(json!({"answer": 42}));
        Ok(result.into())
    }
}

#[tokio::test]
async fn structured_only_result_is_visible_to_model() {
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = StructuredFixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "structured".into(),
        command: "fixture".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    let result = manager
        .execute_tool("mcp__structured__echo", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        result.content.contains("42"),
        "structured result must be visible: {}",
        result.content
    );
    assert_eq!(result.structured_content, Some(json!({"answer":42})));
    manager.disconnect("structured").await.unwrap();
    server.abort();
}
#[tokio::test]
async fn expired_http_session_does_not_replay_tool_call() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let calls = observed.clone();
            tokio::spawn(async move {
                let mut header = Vec::new();
                loop {
                    let Ok(b) = socket.read_u8().await else {
                        return;
                    };
                    header.push(b);
                    if header.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if header.len() > 16384 {
                        return;
                    }
                }
                let head = String::from_utf8(header).unwrap();
                if !head.starts_with("POST ") {
                    return;
                }
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                if socket.read_exact(&mut body).await.is_err() {
                    return;
                }
                let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
                    return;
                };
                if request["method"] == "tools/call" {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                }
                let Some(id) = request.get("id") else {
                    let _ = socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                let result = match request["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                    }
                    Some("tools/list") => {
                        json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                    }
                    _ => return,
                };
                let body = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let manager = McpManager::new();
    manager
        .connect_http(
            &ServerConfig::StreamableHttp {
                id: "expired".into(),
                url: format!("http://{address}/mcp"),
            },
            true,
        )
        .await
        .unwrap();
    assert!(
        manager
            .execute_tool("mcp__expired__echo", json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a failed tool call must not be replayed"
    );
    server.abort();
}
#[tokio::test]
async fn http_configured_header_is_sent_without_being_logged_in_errors() {
    use std::{collections::BTreeMap, sync::Arc};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::Mutex,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let capture = capture.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                loop {
                    let Ok(b) = socket.read_u8().await else {
                        return;
                    };
                    head.push(b);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if head.len() > 16384 {
                        return;
                    }
                }
                let head = String::from_utf8(head).unwrap();
                if !head.starts_with("POST ") {
                    return;
                }
                capture.lock().await.push(head.clone());
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                if socket.read_exact(&mut body).await.is_err() {
                    return;
                }
                let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
                    return;
                };
                let Some(id) = request.get("id") else {
                    let _ = socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                let result = match request["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                    }
                    Some("tools/list") => json!({"tools":[]}),
                    _ => return,
                };
                let body = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let manager = McpManager::new();
    let config = ServerConfig::StreamableHttp {
        id: "headers".into(),
        url: format!("http://{address}/mcp"),
    };
    let headers = BTreeMap::from([("authorization".to_owned(), "Bearer test-secret".to_owned())]);
    manager
        .connect_http_with_headers(&config, &headers, true)
        .await
        .unwrap();
    assert!(seen.lock().await.iter().any(|head| {
        head.to_ascii_lowercase()
            .contains("authorization: bearer test-secret")
    }));
    manager.disconnect("headers").await.unwrap();
    server.abort();
}
#[tokio::test]
async fn configured_tool_filter_does_not_expose_or_execute_denied_tools() {
    use harness_extensions::mcp::ConnectionOptions;
    use std::{collections::BTreeSet, time::Duration};
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = Fixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "filtered".into(),
        command: "fixture".into(),
        args: vec![],
    };
    let options = ConnectionOptions {
        startup_timeout: Duration::from_secs(3),
        tool_timeout: Duration::from_secs(3),
        enabled_tools: None,
        disabled_tools: BTreeSet::from(["echo".to_owned()]),
    };
    manager
        .connect_stdio_with_options(&config, r, w, true, &options)
        .await
        .unwrap();
    assert!(manager.tool_definitions().await.is_empty());
    assert!(
        manager
            .execute_tool("mcp__filtered__echo", json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    manager.disconnect("filtered").await.unwrap();
    server.abort();
}
#[derive(Clone)]
struct LongNameFixture;
impl ServerHandler for LongNameFixture {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        Fixture.get_info()
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let name = format!("long.tool.{}", "a".repeat(100));
        Ok(ListToolsResult {
            tools: vec![
                serde_json::from_value(json!({"name":name,"inputSchema":{"type":"object"}}))
                    .unwrap(),
            ],
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(request.name.to_string())]).into())
    }
}
#[tokio::test]
async fn long_mcp_names_remain_callable_with_provider_compatible_names() {
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = LongNameFixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "server".repeat(15),
        command: "fixture".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    let tools = manager.tool_definitions().await;
    assert_eq!(tools.len(), 1);
    let name = &tools[0].name;
    assert!(name.len() <= 64);
    assert!(
        name.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    );
    let result = manager
        .execute_tool(name, json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(result.content.contains("long.tool."));
    manager.disconnect(config.id()).await.unwrap();
    server.abort();
}
#[derive(Clone)]
struct ImageFixture;
impl ServerHandler for ImageFixture {
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
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::image(
            "large-image-payload",
            "image/png",
        )])
        .into())
    }
}
#[tokio::test]
async fn unsupported_image_is_explained_without_passing_base64_to_model() {
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let service = ImageFixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "image".into(),
        command: "fixture".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    let result = manager
        .execute_tool("mcp__image__echo", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(!result.content.contains("large-image-payload"));
    assert!(result.content.contains("图片"));
    manager.disconnect("image").await.unwrap();
    server.abort();
}
#[tokio::test]
async fn configured_tool_timeout_cancels_without_replaying() {
    use harness_extensions::mcp::ConnectionOptions;
    use std::{
        collections::BTreeSet,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
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
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "timeout".into(),
        command: "fixture".into(),
        args: vec![],
    };
    let options = ConnectionOptions {
        startup_timeout: Duration::from_secs(3),
        tool_timeout: Duration::from_millis(100),
        enabled_tools: None,
        disabled_tools: BTreeSet::new(),
    };
    manager
        .connect_stdio_with_options(&config, r, w, true, &options)
        .await
        .unwrap();
    assert!(
        manager
            .execute_tool("mcp__timeout__echo", json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while !cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 1);
    manager.disconnect("timeout").await.unwrap();
    server.abort();
}
#[tokio::test]
async fn exited_server_no_longer_advertises_tools() {
    let (client, server) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let (sr, sw) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        let service = Fixture.serve((sr, sw)).await.unwrap();
        let _ = service.waiting().await;
    });
    let manager = McpManager::new();
    let config = ServerConfig::Stdio {
        id: "exited".into(),
        command: "fixture".into(),
        args: vec![],
    };
    manager
        .connect_sandboxed_stdio(&config, r, w, true)
        .await
        .unwrap();
    assert_eq!(manager.tool_definitions().await.len(), 1);
    task.abort();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !manager.tool_definitions().await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("closed server must stop advertising tools");
    assert!(!manager.is_connected("exited").await);
    manager.disconnect("exited").await.unwrap();
}
#[tokio::test]
async fn startup_timeout_covers_handshake_and_tool_discovery_together() {
    use harness_extensions::mcp::ConnectionOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut head = Vec::new();
                loop {
                    let Ok(b) = socket.read_u8().await else {
                        return;
                    };
                    head.push(b);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if head.len() > 16384 {
                        return;
                    }
                }
                let head = String::from_utf8(head).unwrap();
                if !head.starts_with("POST ") {
                    return;
                }
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; len];
                if socket.read_exact(&mut body).await.is_err() {
                    return;
                }
                let Ok(req) = serde_json::from_slice::<serde_json::Value>(&body) else {
                    return;
                };
                let Some(id) = req.get("id") else {
                    let _=socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                let result = match req["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":req["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                    }
                    Some("tools/list") => json!({"tools":[]}),
                    _ => return,
                };
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                let body = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    let manager = McpManager::new();
    let config = ServerConfig::StreamableHttp {
        id: "slow".into(),
        url: format!("http://{address}/mcp"),
    };
    let options = ConnectionOptions {
        startup_timeout: std::time::Duration::from_millis(250),
        ..Default::default()
    };
    assert!(
        manager
            .connect_http_with_options(&config, &Default::default(), true, &options)
            .await
            .is_err()
    );
    assert!(manager.tool_definitions().await.is_empty());
    server.abort();
}
