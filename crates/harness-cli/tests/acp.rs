//! ACP 连接层集成测试：duplex 管道 + 真实 Runtime + 本地 mock 模型端点。
//!
//! 测试客户端为手写 ndjson JSON-RPC 脚本，作为协议应答的独立裁判。

use agent_client_protocol::ByteStreams;
use harness_cli::acp::{AcpOptions, serve_transport};
use harness_contracts::ModelConfig;
use harness_runtime::Runtime;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

const DEFAULT_MODEL_KEY: &str = "cli.default_model";

/// 手写 ACP 客户端：记录全部收到的消息，按需等待响应或通知。
struct TestClient {
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    reader: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    log: Arc<Mutex<Vec<Value>>>,
    next_id: u64,
}

impl TestClient {
    fn new(stream: tokio::io::DuplexStream) -> (Self, Arc<Mutex<Vec<Value>>>) {
        let (reader, writer) = tokio::io::split(stream);
        let log = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                writer,
                reader: BufReader::new(reader),
                log: log.clone(),
                next_id: 0,
            },
            log,
        )
    }

    async fn send_raw(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn notify(&mut self, method: &str, params: Value) {
        let line = json!({"jsonrpc":"2.0","method":method,"params":params}).to_string();
        self.send_raw(&line).await;
    }

    /// 发送请求并等待对应响应；期间收到的通知写入日志。
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request_no_wait(method, params).await;
        self.wait_response(id).await
    }

    async fn send_request_no_wait(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        self.send_raw(&line).await;
        id
    }

    async fn wait_response(&mut self, id: u64) -> Value {
        loop {
            let message = self.next_message().await;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return message;
            }
        }
    }

    async fn next_message(&mut self) -> Value {
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.reader.read_line(&mut line),
        )
        .await
        .unwrap_or_else(|_| panic!("等待 ACP 消息超时（30 秒），已收到日志见 log"))
        .unwrap();
        let message: Value = serde_json::from_str(line.trim()).unwrap();
        self.log.lock().await.push(message.clone());
        message
    }

    /// 等待一条匹配标签的 session/update 通知（先查已收日志，再读新消息）。
    async fn until_update(&mut self, tag: &str) -> Value {
        loop {
            {
                let log = self.log.lock().await;
                if let Some(message) = log.iter().find(|m| {
                    m["method"] == "session/update" && m["params"]["update"]["sessionUpdate"] == tag
                }) {
                    return message.clone();
                }
            }
            self.next_message().await;
        }
    }

    /// 等待指定 toolCallId 的 tool_call 通知。
    async fn until_tool_call(&mut self, call_id: &str) -> Value {
        loop {
            {
                let log = self.log.lock().await;
                if let Some(message) = log.iter().find(|m| {
                    m["method"] == "session/update"
                        && m["params"]["update"]["sessionUpdate"] == "tool_call"
                        && m["params"]["update"]["toolCallId"] == call_id
                }) {
                    return message.clone();
                }
            }
            self.next_message().await;
        }
    }

    /// 等待指定 toolCallId 的 tool_call_update 通知。
    async fn until_tool_call_update(&mut self, call_id: &str) -> Value {
        loop {
            {
                let log = self.log.lock().await;
                if let Some(message) = log.iter().find(|m| {
                    m["method"] == "session/update"
                        && m["params"]["update"]["sessionUpdate"] == "tool_call_update"
                        && m["params"]["update"]["toolCallId"] == call_id
                }) {
                    return message.clone();
                }
            }
            self.next_message().await;
        }
    }

    /// 等待一条 agent→client 的方法请求（如 session/request_permission）。
    async fn until_request(&mut self, method: &str) -> Value {
        loop {
            let message = self.next_message().await;
            if message["method"] == method {
                return message;
            }
        }
    }

    /// 应答 agent 的请求。
    async fn respond(&mut self, id: &Value, result: Value) {
        let line = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
        self.send_raw(&line).await;
    }
}

/// 启动 agent 与测试客户端。
async fn setup() -> (
    TestClient,
    Arc<Mutex<Vec<Value>>>,
    Arc<Runtime>,
    std::path::PathBuf,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let (client_io, agent_io) = tokio::io::duplex(1024 * 1024);
    let (agent_read, agent_write) = tokio::io::split(agent_io);
    tokio::spawn(serve_transport(
        runtime.clone(),
        AcpOptions::default(),
        ByteStreams::new(agent_write.compat_write(), agent_read.compat()),
    ));
    let (client, log) = TestClient::new(client_io);
    (client, log, runtime, source, temp)
}

async fn initialize(client: &mut TestClient) -> Value {
    client
        .request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
        .await
}

async fn new_session(client: &mut TestClient, source: &std::path::Path) -> String {
    let response = client
        .request(
            "session/new",
            json!({"cwd": source.to_str().unwrap(), "mcpServers": []}),
        )
        .await;
    assert!(
        response.get("error").is_none(),
        "session/new 失败: {response}"
    );
    response["result"]["sessionId"].as_str().unwrap().to_owned()
}

/// 本地 mock SSE 端点：按序应答预置响应；之后的连接挂住不响应（取消测试用）。
/// 返回 base URL 与“首个请求已在途”信号。
async fn mock_model(
    runtime: &Arc<Runtime>,
    sse_bodies: Vec<String>,
) -> (String, mpsc::Receiver<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        for body in sse_bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = tx.send(()).await;
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buffer = [0u8; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            while bytes.len() < header_end + length {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }
        // 预置响应耗尽后，后续连接挂住不响应（socket 由永不完成的 future 持有）。
        loop {
            let (_socket, _) = listener.accept().await.unwrap();
            let _ = tx.send(()).await;
            std::future::pending::<()>().await;
        }
    });
    let provider: harness_contracts::ProviderInfo = serde_json::from_value(json!({
        "id": "mockprov", "name": "mockprov", "builtin": false,
        "endpoints": [{"apiMode": "chatCompletions", "baseUrl": base}],
        "defaultModel": "mock", "credentialRef": "", "proxy": null
    }))
    .unwrap();
    runtime.models.save_provider(provider, None).await.unwrap();
    let config: ModelConfig = serde_json::from_value(json!({
        "id": "mock", "name": "mock", "baseUrl": base, "model": "mock-model",
        "credentialRef": "", "proxy": null, "providerId": "mockprov",
        "apiMode": "chatCompletions", "maxOutputTokens": 1024
    }))
    .unwrap();
    runtime.models.save_model(config, None).await.unwrap();
    runtime
        .store
        .set(DEFAULT_MODEL_KEY, &json!("mock"))
        .await
        .unwrap();
    (base, rx)
}

fn chat_answer(content: &str, calls: Vec<Value>) -> String {
    let calls = calls
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "index": i, "id": c["id"], "type": "function",
                "function": {"name": c["name"], "arguments": c["arguments"].to_string()}
            })
        })
        .collect::<Vec<_>>();
    // PR #60 起回合要求可识别的 token 用量，SSE 流补一帧 usage 终帧。
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":if calls.is_empty(){content}else{""},"tool_calls":calls},"finish_reason":if calls.is_empty(){"stop"}else{"tool_calls"}}]}),
        json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}})
    )
}

// ---------- 协议生命周期 ----------

#[tokio::test]
async fn initialize_negotiates_and_gates_sessions() {
    let (mut client, _log, _runtime, source, _temp) = setup().await;

    let gated = client
        .request(
            "session/new",
            json!({"cwd": source.to_str().unwrap(), "mcpServers": []}),
        )
        .await;
    assert_eq!(
        gated["error"]["code"], -32600,
        "未 initialize 应被拒绝: {gated}"
    );

    let init = initialize(&mut client).await;
    assert_eq!(init["result"]["protocolVersion"], 1);
    assert_eq!(init["result"]["agentCapabilities"]["loadSession"], true);

    let response = client
        .request(
            "session/new",
            json!({"cwd": source.to_str().unwrap(), "mcpServers": []}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert!(!response["result"]["sessionId"].as_str().unwrap().is_empty());
    assert_eq!(response["result"]["modes"]["currentModeId"], "ask");
    assert_eq!(
        response["result"]["modes"]["availableModes"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[tokio::test]
async fn unhandled_method_answers_32601() {
    let (mut client, _log, _runtime, _source, _temp) = setup().await;
    initialize(&mut client).await;
    let response = client.request("foo/bar", json!({})).await;
    assert_eq!(response["error"]["code"], -32601);
}

#[tokio::test]
async fn set_mode_updates_policy_and_notifies() {
    let (mut client, _log, _runtime, source, _temp) = setup().await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;
    let response = client
        .request(
            "session/set_mode",
            json!({"sessionId": session, "modeId": "auto"}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let update = client.until_update("current_mode_update").await;
    assert_eq!(update["params"]["update"]["currentModeId"], "auto");

    let unknown = client
        .request(
            "session/set_mode",
            json!({"sessionId": session, "modeId": "bogus"}),
        )
        .await;
    assert_eq!(unknown["error"]["code"], -32602);
}

// ---------- prompt：流式与终态 ----------

#[tokio::test]
async fn prompt_streams_chunks_and_ends_with_end_turn() {
    let (mut client, _log, runtime, source, _temp) = setup().await;
    mock_model(&runtime, vec![chat_answer("finished", vec![])]).await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;

    let response = client
        .request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "你好"}]}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["stopReason"], "end_turn");

    let chunk = client.until_update("agent_message_chunk").await;
    assert_eq!(chunk["params"]["update"]["content"]["text"], "finished");
}

#[tokio::test]
async fn prompt_without_model_answers_internal_error() {
    let (mut client, _log, _runtime, source, _temp) = setup().await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;
    let response = client
        .request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "你好"}]}),
        )
        .await;
    assert_eq!(response["error"]["code"], -32603);
    let text = response["error"]["message"].as_str().unwrap();
    assert!(text.contains("模型"), "{text}");
}

#[tokio::test]
async fn unsupported_content_block_answers_invalid_params() {
    let (mut client, _log, _runtime, source, _temp) = setup().await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;
    let response = client
        .request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "image", "data": "x", "mimeType": "image/png"}]}),
        )
        .await;
    assert_eq!(response["error"]["code"], -32602);
}

// ---------- 审批 → session/request_permission ----------

#[tokio::test]
async fn approval_ask_mode_roundtrips_selected_allow() {
    let (mut client, _log, runtime, source, _temp) = setup().await;
    mock_model(
        &runtime,
        vec![
            chat_answer("", vec![json!({"id": "c1", "name": "read", "arguments": {"path": "a.txt"}})]),
            chat_answer(
                "",
                vec![json!({"id": "c2", "name": "write", "arguments": {"path": "a.txt", "content": "new"}})],
            ),
            chat_answer("done", vec![]),
        ],
    )
    .await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;

    // 先发 prompt（不等响应），再处理途中到来的授权请求。
    let prompt_id = client
        .send_request_no_wait(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "改文件"}]}),
        )
        .await;

    // read 免审批直接执行。
    client.until_tool_call("c1").await;
    let read_result = client.until_tool_call_update("c1").await;
    assert_eq!(read_result["params"]["update"]["status"], "completed");

    // write 需审批：先 tool_call（kind=edit），后 request_permission。
    let tool_call = client.until_tool_call("c2").await;
    assert_eq!(tool_call["params"]["update"]["kind"], "edit");

    let permission = client.until_request("session/request_permission").await;
    assert_eq!(permission["params"]["options"][0]["optionId"], "allow");
    assert_eq!(permission["params"]["options"][1]["optionId"], "reject");
    assert_eq!(permission["params"]["toolCall"]["toolCallId"], "c2");
    client
        .respond(
            &permission["id"],
            json!({"outcome": {"outcome": "selected", "optionId": "allow"}}),
        )
        .await;

    let tool_result = client.until_tool_call_update("c2").await;
    assert_eq!(tool_result["params"]["update"]["status"], "completed");

    let response = client.wait_response(prompt_id).await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["stopReason"], "end_turn");
}

#[tokio::test]
async fn cancelled_permission_outcome_denies_tool() {
    let (mut client, _log, runtime, source, _temp) = setup().await;
    mock_model(
        &runtime,
        vec![
            chat_answer(
                "",
                vec![json!({"id": "c1", "name": "write", "arguments": {"path": "a.txt", "content": "new"}})],
            ),
            chat_answer("好的，已取消写入", vec![]),
        ],
    )
    .await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;

    let prompt_id = client
        .send_request_no_wait(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "改文件"}]}),
        )
        .await;
    let _tool_call = client.until_update("tool_call").await;
    let permission = client.until_request("session/request_permission").await;
    client
        .respond(
            &permission["id"],
            json!({"outcome": {"outcome": "cancelled"}}),
        )
        .await;

    let denied = client.until_update("tool_call_update").await;
    assert_eq!(denied["params"]["update"]["status"], "failed");

    let response = client.wait_response(prompt_id).await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
}

// ---------- 取消与回放 ----------

#[tokio::test]
async fn cancel_notification_stops_prompt_with_cancelled() {
    let (mut client, _log, runtime, source, _temp) = setup().await;
    // 零预置响应：模型请求在途即挂住。
    let (_base, mut in_flight) = mock_model(&runtime, vec![]).await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;

    let prompt_id = client
        .send_request_no_wait(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "慢慢来"}]}),
        )
        .await;
    // 等模型请求真正在途，避免取消落在回合开始前。
    tokio::time::timeout(std::time::Duration::from_secs(10), in_flight.recv())
        .await
        .unwrap()
        .unwrap();
    client
        .notify("session/cancel", json!({"sessionId": session}))
        .await;

    let response = client.wait_response(prompt_id).await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["stopReason"], "cancelled");
}

#[tokio::test]
async fn session_load_replays_history_in_order() {
    let (mut client, log, runtime, source, _temp) = setup().await;
    initialize(&mut client).await;
    let session = new_session(&mut client, &source).await;

    runtime
        .store
        .append(
            &session,
            "user.message",
            json!({"content": "问", "attachments": []}),
        )
        .await
        .unwrap();
    runtime
        .store
        .append(
            &session,
            "assistant.message",
            json!({"content": "答", "reasoning": "思", "toolCalls": [], "usage": null, "replay": null}),
        )
        .await
        .unwrap();
    runtime
        .store
        .append(
            &session,
            "tool.call",
            json!({"id": "c1", "name": "read", "arguments": {"path": "a.txt"}}),
        )
        .await
        .unwrap();
    runtime
        .store
        .append(
            &session,
            "tool.result",
            json!({"callId": "c1", "name": "read", "content": "old", "isError": false, "status": "completed"}),
        )
        .await
        .unwrap();

    let response = client
        .request(
            "session/load",
            json!({"sessionId": session, "cwd": source.to_str().unwrap(), "mcpServers": []}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["modes"]["currentModeId"], "ask");

    let tags: Vec<String> = log
        .lock()
        .await
        .iter()
        .filter(|m| m["method"] == "session/update")
        .map(|m| {
            m["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        tags,
        vec![
            "user_message_chunk",
            "agent_thought_chunk",
            "agent_message_chunk",
            "tool_call",
            "tool_call_update"
        ]
    );
    // tool_call_update 应为终态 completed。
    let log = log.lock().await;
    let result_update = log
        .iter()
        .find(|m| {
            m["method"] == "session/update"
                && m["params"]["update"]["sessionUpdate"] == "tool_call_update"
        })
        .unwrap();
    assert_eq!(result_update["params"]["update"]["status"], "completed");
}

// ---------- 真实子进程冒烟（Stdio 传输） ----------

#[test]
fn acp_subprocess_serves_initialize_and_new_session() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "--data-dir",
            dir.path().join("host").to_str().unwrap(),
            "acp",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());

    let mut send = |line: String| {
        input.write_all(line.as_bytes()).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    };
    send(
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {}}
        })
        .to_string(),
    );
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let init: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], 1);
    assert_eq!(init["result"]["agentCapabilities"]["loadSession"], true);

    send(
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "session/new",
            "params": {"cwd": source.to_str().unwrap(), "mcpServers": []}
        })
        .to_string(),
    );
    line.clear();
    out.read_line(&mut line).unwrap();
    let created: Value = serde_json::from_str(line.trim()).unwrap();
    assert!(
        created.get("error").is_none(),
        "session/new 失败: {created}"
    );
    assert!(!created["result"]["sessionId"].as_str().unwrap().is_empty());

    drop(input);
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
async fn preset_sets_initial_mode_for_new_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    runtime
        .presets
        .save(harness_contracts::PresetConfig {
            id: "auto-run".into(),
            name: "自动".into(),
            description: None,
            model_id: None,
            effort: None,
            approval: Some("auto".into()),
        })
        .await
        .unwrap();
    let (client_io, agent_io) = tokio::io::duplex(1024 * 1024);
    let (agent_read, agent_write) = tokio::io::split(agent_io);
    tokio::spawn(serve_transport(
        runtime,
        AcpOptions {
            preset: Some("auto-run".into()),
            ..Default::default()
        },
        ByteStreams::new(agent_write.compat_write(), agent_read.compat()),
    ));
    let (mut client, _log) = TestClient::new(client_io);
    initialize(&mut client).await;
    let response = client
        .request(
            "session/new",
            json!({"cwd": source.to_str().unwrap(), "mcpServers": []}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"]["modes"]["currentModeId"], "auto",
        "预设审批档位作为新会话初始模式"
    );
}

#[tokio::test]
async fn unknown_preset_fails_startup() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let (client_io, agent_io) = tokio::io::duplex(64 * 1024);
    let (agent_read, agent_write) = tokio::io::split(agent_io);
    let outcome = serve_transport(
        runtime,
        AcpOptions {
            preset: Some("ghost".into()),
            ..Default::default()
        },
        ByteStreams::new(agent_write.compat_write(), agent_read.compat()),
    )
    .await;
    let error = outcome.unwrap_err();
    assert_eq!(error.code, "PRESET_NOT_FOUND", "未知预设启动即失败");
    drop(client_io);
}
