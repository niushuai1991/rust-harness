use harness_contracts::ExecutionMode;
use harness_extensions::mcp::ServerConfig;
use harness_runtime::Runtime;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

async fn server(label: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut head = Vec::new();
                loop {
                    let Ok(byte) = socket.read_u8().await else {
                        return;
                    };
                    head.push(byte);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if head.len() > 16384 {
                        return;
                    }
                }
                let head = String::from_utf8(head).unwrap();
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; len];
                if socket.read_exact(&mut body).await.is_err() {
                    return;
                }
                if !head.starts_with("POST ") {
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
                        json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":label,"version":"1"}})
                    }
                    Some("tools/list") => {
                        json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                    }
                    Some("tools/call") => {
                        json!({"content":[{"type":"text","text":label}],"structuredContent":{"detail":label}})
                    }
                    Some("resources/list") => {
                        json!({"resources":[{"uri":format!("fixture://{label}"),"name":label}]})
                    }
                    _ => return,
                };
                let data = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    data.len(),
                    data
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}/mcp"), task)
}

#[tokio::test]
async fn same_named_servers_are_isolated_by_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let a = runtime
        .workspaces
        .create(&a, None, ExecutionMode::Direct)
        .unwrap();
    let b = runtime
        .workspaces
        .create(&b, None, ExecutionMode::Direct)
        .unwrap();
    let (url_a, server_a) = server("first").await;
    let (url_b, server_b) = server("second").await;
    runtime
        .connect_mcp(
            &a.id,
            ServerConfig::StreamableHttp {
                id: "shared".into(),
                url: url_a,
            },
        )
        .await
        .unwrap();
    let url_b_for_reconnect = url_b.clone();
    runtime
        .connect_mcp(
            &b.id,
            ServerConfig::StreamableHttp {
                id: "shared".into(),
                url: url_b,
            },
        )
        .await
        .unwrap();
    let tools_a = runtime.scoped_mcp_tools(&a.id).await;
    let tools_b = runtime.scoped_mcp_tools(&b.id).await;
    assert_eq!(tools_a.len(), 1);
    assert_eq!(tools_b.len(), 1);
    assert!(
        tools_a[0].name.contains("shared"),
        "server name should remain recognizable"
    );
    assert_ne!(tools_a[0].name, tools_b[0].name);
    let ra = runtime
        .call_scoped_mcp(&a.id, &tools_a[0].name, json!({}), CancellationToken::new())
        .await
        .unwrap();
    let rb = runtime
        .call_scoped_mcp(&b.id, &tools_b[0].name, json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert!(ra.content.contains("first"));
    assert!(rb.content.contains("second"));
    assert!(
        runtime
            .call_scoped_mcp(&b.id, &tools_a[0].name, json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    let resources = runtime
        .scoped_mcp_resources(&a.id, "shared", None)
        .await
        .unwrap();
    assert!(resources.to_string().contains("first"));
    assert!(!resources.to_string().contains("second"));
    runtime
        .disconnect_scoped_mcp(&a.id, "shared")
        .await
        .unwrap();
    runtime
        .connect_mcp(
            &a.id,
            ServerConfig::StreamableHttp {
                id: "shared".into(),
                url: url_b_for_reconnect,
            },
        )
        .await
        .unwrap();
    assert!(
        runtime
            .call_scoped_mcp(&a.id, &tools_a[0].name, json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(runtime.scoped_mcp_tools(&b.id).await.len(), 1);
    server_a.abort();
    server_b.abort();
}
#[tokio::test]
async fn session_connections_are_not_shared_with_other_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let first = runtime
        .store
        .create_session(&workspace.id, "first")
        .await
        .unwrap();
    let second = runtime
        .store
        .create_session(&workspace.id, "second")
        .await
        .unwrap();
    let (url, server) = server("session").await;
    runtime
        .connect_session_mcp(
            &first.id,
            ServerConfig::StreamableHttp {
                id: "shared".into(),
                url,
            },
        )
        .await
        .unwrap();
    let records = runtime.mcp_connections().await;
    assert!(
        records
            .iter()
            .any(|entry| entry["sessionId"] == first.id && entry["status"] == "connected")
    );
    let available = runtime.session_mcp_tools(&first.id).await.unwrap();
    assert_eq!(available.len(), 1);
    assert!(
        runtime
            .session_mcp_tools(&second.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        runtime
            .call_session_mcp(
                &second.id,
                &available[0].name,
                json!({}),
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .disconnect_session_mcp(&second.id, "shared")
            .await
            .is_err()
    );
    assert_eq!(runtime.session_mcp_tools(&first.id).await.unwrap().len(), 1);
    runtime
        .disconnect_session_mcp(&first.id, "shared")
        .await
        .unwrap();
    assert!(
        runtime
            .session_mcp_tools(&first.id)
            .await
            .unwrap()
            .is_empty()
    );
    server.abort();
}
#[tokio::test]
async fn session_connect_requires_trusted_enabled_configuration() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("configured").await;
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"remote","transport":{"type":"streamableHttp","url":url}
    }))
    .unwrap();
    let scope = McpScope::Project(workspace.id.clone());
    runtime
        .mcp_config
        .save(scope.clone(), config.clone())
        .await
        .unwrap();
    assert!(
        runtime
            .connect_saved_session_mcp(&session.id, "remote")
            .await
            .is_err()
    );
    runtime
        .mcp_config
        .trust_exact(scope, &config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    let tool = runtime
        .session_mcp_tools(&session.id)
        .await
        .unwrap()
        .remove(0);
    let mut modified = config.clone();
    modified.enabled = false;
    runtime
        .mcp_config
        .save(McpScope::Project(workspace.id.clone()), modified)
        .await
        .unwrap();
    assert!(
        runtime
            .session_mcp_tools(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        runtime
            .call_session_mcp(&session.id, &tool.name, json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    server.abort();
}
#[tokio::test]
async fn saved_server_applies_tool_allow_list() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("configured").await;
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"remote","enabledTools":["other"],"transport":{"type":"streamableHttp","url":url}
    }))
    .unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    assert!(
        runtime
            .session_mcp_tools(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    assert_eq!(
        runtime.session_mcp_status(&session.id).await.unwrap()[0]["status"],
        "disconnected"
    );
    server.abort();
}
#[tokio::test]
async fn saved_http_server_resolves_configured_header_reference() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    assert!(std::env::var_os("USERPROFILE").is_some());
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("configured").await;
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"remote","transport":{"type":"streamableHttp","url":url,
            "headers":{"x-reference":{"source":"environment","name":"USERPROFILE"}}}
    }))
    .unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    assert_eq!(
        runtime.session_mcp_tools(&session.id).await.unwrap().len(),
        1
    );
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    server.abort();
}
#[tokio::test]
async fn changed_config_during_handshake_must_not_publish_old_connection() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    use std::sync::Arc;
    use tokio::sync::Notify;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let server_started = started.clone();
    let server_release = release.clone();
    let stalled_server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let started = server_started.clone();
            let release = server_release.clone();
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
                        started.notify_one();
                        release.notified().await;
                        json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                    }
                    Some("tools/list") => {
                        json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                    }
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
    let config: McpServerConfig = serde_json::from_value(json!({"id":"remote","transport":{"type":"streamableHttp","url":format!("http://{address}/mcp")}})).unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, config.clone())
        .await
        .unwrap();
    let clone = runtime.clone();
    let sid = session.id.clone();
    let connection =
        tokio::spawn(async move { clone.connect_saved_session_mcp(&sid, "remote").await });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    assert_eq!(
        runtime
            .session_plan_set(&session.id, true)
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_BUSY"
    );
    let (other_url, other_server) = server("other").await;
    let other: McpServerConfig = serde_json::from_value(
        json!({"id":"other","transport":{"type":"streamableHttp","url":other_url}}),
    )
    .unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, other)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.connect_saved_session_mcp(&session.id, "other"),
    )
    .await
    .expect("slow server must not block unrelated connection")
    .unwrap();
    let mut changed = config;
    changed.enabled = false;
    runtime
        .mcp_config
        .save(McpScope::Global, changed)
        .await
        .unwrap();
    release.notify_one();
    assert!(connection.await.unwrap().is_err());
    assert_eq!(
        runtime.session_mcp_tools(&session.id).await.unwrap().len(),
        1
    );
    runtime
        .disconnect_session_mcp(&session.id, "other")
        .await
        .unwrap();
    other_server.abort();
    stalled_server.abort();
}
#[cfg(windows)]
#[tokio::test]
async fn saved_stdio_server_uses_session_root_subdirectory_and_explicit_environment() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    let script = source.join("fixture.py");
    std::fs::write(&script, r#"
import json, os, sys
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req: continue
    method = req["method"]
    if method == "initialize":
        result = {"protocolVersion":req["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}
    elif method == "tools/list":
        result = {"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}
    elif method == "tools/call":
        result = {"content":[{"type":"text","text":os.getcwd() + "|" + os.environ.get("MCP_VALUE","missing")}]}
    else: continue
    print(json.dumps({"jsonrpc":"2.0","id":req["id"],"result":result}),flush=True)
"#).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"local","transport":{"type":"stdio","command":"python.exe","args":[script.to_string_lossy()],
            "cwd":"sub","env":{"MCP_VALUE":{"source":"environment","name":"USERPROFILE"}}}
    })).unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "local")
        .await
        .unwrap();
    let tools = runtime.session_mcp_tools(&session.id).await.unwrap();
    let result = runtime
        .call_session_mcp(
            &session.id,
            &tools[0].name,
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result.content.contains("sub"));
    assert!(
        serde_json::from_str::<serde_json::Value>(&result.content).unwrap()[0]["text"]
            .as_str()
            .unwrap()
            .contains(&std::env::var("USERPROFILE").unwrap())
    );
    runtime
        .disconnect_session_mcp(&session.id, "local")
        .await
        .unwrap();
}
#[tokio::test]
async fn saving_a_changed_config_closes_its_existing_session_connection() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("configured").await;
    let mut config: McpServerConfig = serde_json::from_value(
        json!({"id":"remote","transport":{"type":"streamableHttp","url":url}}),
    )
    .unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config.clone())
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    config.enabled = false;
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    assert!(
        runtime
            .session_mcp_tools(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        runtime
            .disconnect_session_mcp(&session.id, "remote")
            .await
            .is_err()
    );
    server.abort();
}
#[tokio::test]
async fn legacy_workspace_connection_is_not_exposed_to_session_model() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ToolDefinition};
    use harness_runtime::model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider};
    use std::sync::{Arc, Mutex};
    struct Probe(Arc<Mutex<Vec<ToolDefinition>>>);
    #[async_trait]
    impl ModelProvider for Probe {
        async fn complete(
            &self,
            _: Vec<ModelMessage>,
            tools: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            *self.0.lock().unwrap() = tools;
            Ok(ModelAnswer {
                content: "ok".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":0}),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("legacy").await;
    runtime
        .connect_mcp(
            &workspace.id,
            ServerConfig::StreamableHttp {
                id: "shared".into(),
                url,
            },
        )
        .await
        .unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    runtime
        .clone()
        .run_session(session.id, "test".into(), Arc::new(Probe(captured.clone())))
        .await
        .unwrap();
    assert!(
        !captured
            .lock()
            .unwrap()
            .iter()
            .any(|tool| tool.name.starts_with("mcp__"))
    );
    server.abort();
}
#[tokio::test]
async fn session_model_can_discover_and_call_its_saved_mcp_tool() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ApprovalPolicy, ToolCall, ToolDefinition};
    use harness_runtime::{
        mcp_config::{McpScope, McpServerConfig},
        model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Script(AtomicUsize);
    #[async_trait]
    impl ModelProvider for Script {
        async fn complete(
            &self,
            messages: Vec<ModelMessage>,
            tools: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            let usage = json!({"prompt_tokens":0,"completion_tokens":0});
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                let tool = tools
                    .iter()
                    .find(|tool| tool.name.starts_with("mcp__"))
                    .expect("MCP tool must be visible");
                Ok(ModelAnswer {
                    calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: tool.name.clone(),
                        arguments: json!({}),
                    }],
                    usage,
                    ..Default::default()
                })
            } else {
                assert!(format!("{messages:?}").contains("end-to-end"));
                Ok(ModelAnswer {
                    content: "done".into(),
                    usage,
                    ..Default::default()
                })
            }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    runtime.set_default_approval_policy(Some(ApprovalPolicy::Auto));
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("end-to-end").await;
    let config: McpServerConfig = serde_json::from_value(
        json!({"id":"remote","transport":{"type":"streamableHttp","url":url}}),
    )
    .unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "test".into(),
            Arc::new(Script(AtomicUsize::new(0))),
        )
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|event| {
        event.kind == "tool.result"
            && event.data["content"]
                .as_str()
                .is_some_and(|text| text.contains("end-to-end"))
    }));
    assert!(events.iter().any(|event| event.kind == "tool.result"
        && event.data["structuredContent"]["detail"] == "end-to-end"));
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    server.abort();
}
#[cfg(windows)]
#[tokio::test]
async fn scratch_session_runs_saved_stdio_server_inside_its_own_root() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "scratch").await.unwrap();
    let root = runtime.workspaces.ensure_scratch_root(&session.id).unwrap();
    let script = root.join("fixture.py");
    std::fs::write(&script, r#"
import json, os, sys
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req: continue
    method = req["method"]
    if method == "initialize":
        result = {"protocolVersion":req["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}
    elif method == "tools/list":
        result = {"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}
    elif method == "tools/call":
        result = {"content":[{"type":"text","text":os.getcwd()}]}
    else: continue
    print(json.dumps({"jsonrpc":"2.0","id":req["id"],"result":result}),flush=True)
"#).unwrap();
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"local","transport":{"type":"stdio","command":"python.exe","args":[script.to_string_lossy()]}
    })).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "local")
        .await
        .unwrap();
    let tools = runtime.session_mcp_tools(&session.id).await.unwrap();
    let result = runtime
        .call_session_mcp(
            &session.id,
            &tools[0].name,
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        std::fs::canonicalize(data[0]["text"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(root).unwrap()
    );
    runtime
        .disconnect_session_mcp(&session.id, "local")
        .await
        .unwrap();
}
#[cfg(windows)]
#[tokio::test]
async fn replica_and_worktree_stdio_use_their_actual_session_roots() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    use std::process::Command;
    for mode in [ExecutionMode::Replica, ExecutionMode::Worktree] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("fixture.py"), r#"
import json, os, sys
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req: continue
    method = req["method"]
    if method == "initialize":
        result = {"protocolVersion":req["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}
    elif method == "tools/list": result = {"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}
    elif method == "tools/call": result = {"content":[{"type":"text","text":os.getcwd()}]}
    else: continue
    print(json.dumps({"jsonrpc":"2.0","id":req["id"],"result":result}),flush=True)
"#).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "fixture.py"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
        let workspace = runtime
            .workspaces
            .create(
                &source,
                None,
                if mode == ExecutionMode::Replica {
                    ExecutionMode::Replica
                } else {
                    ExecutionMode::Direct
                },
            )
            .unwrap();
        let session = runtime
            .store
            .create_session_with_mode(&workspace.id, "session", Some(mode))
            .await
            .unwrap();
        let target = runtime
            .workspaces
            .execution_target(&workspace.id, &session.id, Some(mode), None)
            .unwrap();
        let config: McpServerConfig = serde_json::from_value(json!({"id":"local","transport":{"type":"stdio","command":"python.exe","args":["fixture.py"]}})).unwrap();
        runtime
            .save_mcp_config(McpScope::Global, config)
            .await
            .unwrap();
        runtime
            .connect_saved_session_mcp(&session.id, "local")
            .await
            .unwrap();
        let tools = runtime.session_mcp_tools(&session.id).await.unwrap();
        let result = runtime
            .call_session_mcp(
                &session.id,
                &tools[0].name,
                json!({}),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            std::fs::canonicalize(data[0]["text"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(target.root).unwrap()
        );
        runtime
            .disconnect_session_mcp(&session.id, "local")
            .await
            .unwrap();
    }
}
#[cfg(windows)]
#[tokio::test]
async fn exited_stdio_server_reconnects_on_next_turn_without_replaying_call() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ToolDefinition};
    use harness_runtime::{
        mcp_config::{McpScope, McpServerConfig},
        model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
    };
    use std::sync::{Arc, Mutex};
    struct Probe(Arc<Mutex<bool>>);
    #[async_trait]
    impl ModelProvider for Probe {
        async fn complete(
            &self,
            _: Vec<ModelMessage>,
            tools: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            *self.0.lock().unwrap() = tools.iter().any(|tool| tool.name.starts_with("mcp__"));
            Ok(ModelAnswer {
                content: "ok".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":0}),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("fixture.py"),r#"
import json, sys
for line in sys.stdin:
    req=json.loads(line)
    if "id" not in req: continue
    method=req["method"]
    if method=="initialize": result={"protocolVersion":req["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}
    elif method=="tools/list": result={"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}
    elif method=="tools/call": sys.exit(0)
    else: continue
    print(json.dumps({"jsonrpc":"2.0","id":req["id"],"result":result}),flush=True)
"#).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config:McpServerConfig=serde_json::from_value(json!({"id":"local","transport":{"type":"stdio","command":"python.exe","args":["fixture.py"]}})).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "local")
        .await
        .unwrap();
    let old = runtime.session_mcp_tools(&session.id).await.unwrap()[0]
        .name
        .clone();
    assert!(
        runtime
            .call_session_mcp(&session.id, &old, json!({}), CancellationToken::new())
            .await
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !runtime
            .mcp_connections()
            .await
            .iter()
            .any(|record| record["status"] == "disconnected")
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let available = Arc::new(Mutex::new(false));
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "test".into(),
            Arc::new(Probe(available.clone())),
        )
        .await
        .unwrap();
    assert!(*available.lock().unwrap());
    let new = runtime.session_mcp_tools(&session.id).await.unwrap()[0]
        .name
        .clone();
    assert_ne!(old, new);
    runtime
        .disconnect_session_mcp(&session.id, "local")
        .await
        .unwrap();
}
#[tokio::test]
async fn cancelling_turn_interrupts_server_startup_without_waiting_for_timeout() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ToolDefinition};
    use harness_runtime::{
        mcp_config::{McpScope, McpServerConfig},
        model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
    };
    use std::sync::Arc;
    struct Probe;
    #[async_trait]
    impl ModelProvider for Probe {
        async fn complete(
            &self,
            _: Vec<ModelMessage>,
            _: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            Ok(ModelAnswer {
                content: "ok".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":0}),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 1024];
        let _ = socket.read(&mut buf).await;
        signal.notify_one();
        std::future::pending::<()>().await;
    });
    let config:McpServerConfig=serde_json::from_value(json!({"id":"slow","transport":{"type":"streamableHttp","url":format!("http://{address}/mcp")}})).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    let run = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "test".into(),
        Arc::new(Probe),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    runtime.cancel(&session.id);
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), run)
        .await
        .expect("cancellation must not wait for server startup")
        .unwrap();
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    server.abort();
}
#[tokio::test]
async fn remote_connection_does_not_hold_workspace_filesystem_lease() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("remote").await;
    runtime
        .connect_session_mcp(
            &session.id,
            ServerConfig::StreamableHttp {
                id: "remote".into(),
                url,
            },
        )
        .await
        .unwrap();
    runtime
        .workspaces
        .rename(&workspace.id, "renamed".into())
        .expect("remote-only connection must not lock workspace");
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    server.abort();
}
#[tokio::test]
async fn failed_startup_reports_server_status_without_secret_values() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config:McpServerConfig=serde_json::from_value(json!({"id":"unreachable","startupTimeoutSecs":1,"transport":{"type":"streamableHttp","url":"http://127.0.0.1:1/mcp"}})).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    assert!(
        runtime
            .connect_saved_session_mcp(&session.id, "unreachable")
            .await
            .is_err()
    );
    let statuses = runtime.session_mcp_status(&session.id).await.unwrap();
    assert!(
        statuses
            .iter()
            .any(|status| status["id"] == "unreachable" && status["status"] == "failed")
    );
}
#[cfg(windows)]
#[tokio::test]
async fn cancelling_stalled_stdio_startup_terminates_the_server_process() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ToolDefinition};
    use harness_runtime::{
        mcp_config::{McpScope, McpServerConfig},
        model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
    };
    use std::{process::Command, sync::Arc};
    struct Probe;
    #[async_trait]
    impl ModelProvider for Probe {
        async fn complete(
            &self,
            _: Vec<ModelMessage>,
            _: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            Ok(ModelAnswer {
                content: "ok".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":0}),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("fixture.py"),
        "import os,time
open('pid.txt','w').write(str(os.getpid()))
time.sleep(30)
",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config:McpServerConfig=serde_json::from_value(json!({"id":"slow","transport":{"type":"stdio","command":"python.exe","args":["fixture.py"]}})).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    let run = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "test".into(),
        Arc::new(Probe),
    ));
    let pid_file = source.join("pid.txt");
    let pid: u32 = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(pid) = text.parse()
            {
                break pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    runtime.cancel(&session.id);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let is_gone=Command::new("powershell.exe").args(["-NoProfile","-Command",&format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}")]).status().unwrap().success();
    assert!(
        is_gone,
        "cancelled startup must terminate its server process"
    );
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "需要设置 MCP_FILESYSTEM_SERVER 指向已安装的文件系统服务器入口"]
async fn third_party_filesystem_server_interoperability() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    let script = std::env::var("MCP_FILESYSTEM_SERVER").expect("server path required");
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("hello.txt"), "third-party interoperability").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config: McpServerConfig = serde_json::from_value(json!({
        "id":"filesystem","transport":{"type":"stdio","command":"node.exe","args":[script, source.to_string_lossy()]}
    })).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "filesystem")
        .await
        .unwrap();
    let tools = runtime.session_mcp_tools(&session.id).await.unwrap();
    let read = tools
        .iter()
        .find(|tool| tool.description.contains(" / read_text_file: "))
        .expect("real server must expose read_text_file");
    let result = runtime
        .call_session_mcp(
            &session.id,
            &read.name,
            json!({"path":source.join("hello.txt").to_string_lossy()}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("third-party interoperability"));
    runtime
        .disconnect_session_mcp(&session.id, "filesystem")
        .await
        .unwrap();
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "需要设置 MCP_HTTP_INTEROP_MODULES 指向包含 MCP SDK 的依赖目录"]
async fn sdk_streamable_http_interoperability() {
    use harness_runtime::mcp_config::{McpScope, McpServerConfig};
    use tokio::io::{AsyncBufReadExt, BufReader};
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_http_sdk.mjs");
    let mut child = tokio::process::Command::new("node.exe")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut port = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        reader.read_line(&mut port),
    )
    .await
    .unwrap()
    .unwrap();
    let port: u16 = port
        .trim()
        .parse()
        .expect("server must print listening port");
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let config: McpServerConfig = serde_json::from_value(json!({"id":"http","transport":{"type":"streamableHttp","url":format!("http://127.0.0.1:{port}/mcp")}})).unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime
        .connect_saved_session_mcp(&session.id, "http")
        .await
        .unwrap();
    let tools = runtime.session_mcp_tools(&session.id).await.unwrap();
    let echo = tools
        .iter()
        .find(|tool| tool.description.contains(" / echo: "))
        .unwrap();
    let result = runtime
        .call_session_mcp(
            &session.id,
            &echo.name,
            json!({"text":"http interoperability"}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("http interoperability"));
    runtime
        .disconnect_session_mcp(&session.id, "http")
        .await
        .unwrap();
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
#[tokio::test]
async fn plan_mode_does_not_start_configured_servers() {
    use async_trait::async_trait;
    use harness_contracts::{AppResult, ToolDefinition};
    use harness_runtime::{
        mcp_config::{McpScope, McpServerConfig},
        model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
    };
    use std::sync::Arc;
    struct Probe;
    #[async_trait]
    impl ModelProvider for Probe {
        async fn complete(
            &self,
            _: Vec<ModelMessage>,
            tools: Vec<ToolDefinition>,
            _: CancellationToken,
            _: DeltaSink,
        ) -> AppResult<ModelAnswer> {
            assert!(!tools.iter().any(|tool| tool.name.starts_with("mcp__")));
            Ok(ModelAnswer {
                content: "ok".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":0}),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("unused").await;
    let config: McpServerConfig = serde_json::from_value(
        json!({"id":"remote","transport":{"type":"streamableHttp","url":url}}),
    )
    .unwrap();
    runtime
        .save_mcp_config(McpScope::Global, config)
        .await
        .unwrap();
    runtime.session_plan_set(&session.id, true).await.unwrap();
    assert!(
        runtime
            .connect_saved_session_mcp(&session.id, "remote")
            .await
            .is_err()
    );
    runtime
        .clone()
        .run_session(session.id, "test".into(), Arc::new(Probe))
        .await
        .unwrap();
    assert!(runtime.mcp_connections().await.is_empty());
    server.abort();
}
#[tokio::test]
async fn cannot_enter_plan_mode_with_live_session_server() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    std::fs::create_dir_all(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "session")
        .await
        .unwrap();
    let (url, server) = server("fixture").await;
    runtime
        .connect_session_mcp(
            &session.id,
            ServerConfig::StreamableHttp {
                id: "remote".into(),
                url,
            },
        )
        .await
        .unwrap();
    assert!(runtime.session_plan_set(&session.id, true).await.is_err());
    runtime
        .disconnect_session_mcp(&session.id, "remote")
        .await
        .unwrap();
    runtime.session_plan_set(&session.id, true).await.unwrap();
    server.abort();
}
