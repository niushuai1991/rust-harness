use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use harness_contracts::*;
use harness_extensions::lsp::FrameCodec;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::{
    codec::{FramedRead, FramedWrite},
    sync::CancellationToken,
};

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let _ = messages;
        let mut answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = serde_json::json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}
fn call(id: &str, name: &str, args: Value) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }],
        ..Default::default()
    }
}
fn done() -> ModelAnswer {
    ModelAnswer {
        content: "finished".into(),
        ..Default::default()
    }
}

async fn serve_mock(stream: tokio::io::DuplexStream) {
    let (read, write) = tokio::io::split(stream);
    let mut reader = FramedRead::new(read, FrameCodec::new());
    let mut writer = FramedWrite::new(write, FrameCodec::new());
    while let Some(Ok(message)) = reader.next().await {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned().filter(|v| !v.is_null());
        if let Some(id) = id.filter(|_| !method.is_empty()) {
            let result = match method {
                "initialize" => json!({"capabilities": {"hoverProvider": true}}),
                "textDocument/hover" => {
                    json!({"contents": {"kind": "markdown", "value": "MOCK HOVER"}})
                }
                "workspace/symbol" => json!([
                    {"name": message["params"]["query"], "kind": 12,
                     "location": {"uri": "file:///mock", "range": {
                        "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}}}}
                ]),
                _ => {
                    let _ = writer
                        .send(json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32601, "message": "MethodNotFound"}
                        }))
                        .await;
                    continue;
                }
            };
            let _ = writer
                .send(json!({"jsonrpc": "2.0", "id": id, "result": result}))
                .await;
        } else if method == "textDocument/didOpen" || method == "textDocument/didChange" {
            let uri = message["params"]["textDocument"]["uri"]
                .as_str()
                .unwrap_or("");
            let _ = writer
                .send(json!({
                    "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                    "params": {"uri": uri, "diagnostics": [{
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                        "severity": 1, "source": "mock", "message": "MOCK DIAG"
                    }]}
                }))
                .await;
        }
    }
}

fn mock_config() -> LspServerConfig {
    LspServerConfig {
        id: "rust-analyzer".into(),
        command: "mock".into(),
        args: vec![],
        extensions: vec!["rs".into()],
        initialization_options: Value::Null,
    }
}

async fn connect_mock(runtime: &Runtime, workspace: &str) {
    let (client, server) = tokio::io::duplex(65536);
    tokio::spawn(serve_mock(server));
    let (read, write) = tokio::io::split(client);
    runtime
        .connect_lsp_streams(workspace, &mock_config(), read, write)
        .await
        .unwrap();
}

async fn tool_result(runtime: &Runtime, session: &str, name: &str) -> SessionEvent {
    for _ in 0..500 {
        let events = runtime.store.events(session, 0).await.unwrap();
        if let Some(event) = events
            .iter()
            .rev()
            .find(|e| e.kind == "tool.result" && e.data["name"] == name)
        {
            return event.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no tool.result for {name}");
}

#[tokio::test]
async fn lsp_hover_is_routed_and_exempt_from_approval() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.rs"), "fn main() {}").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    connect_mock(&runtime, &workspace.id).await;
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "lsp_hover",
                json!({"path":"a.rs","line":1,"character":5}),
            ),
            done(),
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "hover".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    assert!(
        runtime.approvals.list().is_empty(),
        "read-only LSP tools must not require approval"
    );
    let result = tool_result(&runtime, &session.id, "lsp_hover").await;
    assert_eq!(result.data["isError"], false);
    assert!(
        result.data["content"]
            .as_str()
            .unwrap()
            .contains("MOCK HOVER")
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let tool_messages = events
        .iter()
        .filter(|e| e.kind == "tool.result")
        .filter(|e| e.data["name"] == "lsp_hover")
        .count();
    assert_eq!(tool_messages, 1);
}

#[tokio::test]
async fn lsp_diagnostics_waits_and_aggregates_workspace_cache() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.rs"), "fn main() {}").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    connect_mock(&runtime, &workspace.id).await;
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "lsp_diagnostics", json!({"path":"a.rs","wait":true})),
            call("c2", "lsp_diagnostics", json!({})),
            done(),
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "diagnostics".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    let first = tool_result(&runtime, &session.id, "lsp_diagnostics").await;
    assert!(
        first.data["content"]
            .as_str()
            .unwrap()
            .contains("MOCK DIAG")
    );
    let second = tool_result(&runtime, &session.id, "lsp_diagnostics").await;
    assert!(
        second.data["content"]
            .as_str()
            .unwrap()
            .contains("MOCK DIAG")
    );
}

#[tokio::test]
async fn lsp_symbols_support_document_and_workspace_modes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.rs"), "fn main() {}").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    connect_mock(&runtime, &workspace.id).await;
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "lsp_symbols", json!({"query":"main"})),
            done(),
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "symbols".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    let result = tool_result(&runtime, &session.id, "lsp_symbols").await;
    assert_eq!(result.data["isError"], false);
    assert!(result.data["content"].as_str().unwrap().contains("main"));
}

#[tokio::test]
async fn lsp_unknown_extension_reports_missing_server() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.py"), "print(1)").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "lsp_hover",
                json!({"path":"a.py","line":1,"character":1}),
            ),
            done(),
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "no server".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    let result = tool_result(&runtime, &session.id, "lsp_hover").await;
    assert_eq!(result.data["isError"], true);
    let content = result.data["content"].as_str().unwrap();
    assert!(content.contains("LSP_NO_SERVER"), "{content}");
    assert!(content.contains("py"), "{content}");
}

#[tokio::test]
async fn lsp_spawn_failure_reports_configuration_hint() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.rs"), "fn main() {}").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    runtime
        .store
        .set(
            "lsp.servers.v1",
            &json!([{
                "id": "rust-analyzer",
                "command": "definitely-not-a-real-server-xyz",
                "args": [],
                "extensions": ["rs"]
            }]),
        )
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "lsp_hover",
                json!({"path":"a.rs","line":1,"character":1}),
            ),
            done(),
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "spawn fail".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    let result = tool_result(&runtime, &session.id, "lsp_hover").await;
    assert_eq!(result.data["isError"], true);
    let content = result.data["content"].as_str().unwrap();
    assert!(content.contains("PROCESS_NOT_FOUND"), "{content}");
    assert!(content.contains("lsp.servers.v1"), "{content}");
}

#[tokio::test]
async fn lsp_definitions_are_exposed_and_restart_rejects_unknown_servers() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let names: Vec<String> = harness_runtime::agent::builtin_tools()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for name in [
        "lsp_diagnostics",
        "lsp_hover",
        "lsp_definition",
        "lsp_references",
        "lsp_symbols",
    ] {
        assert!(names.iter().any(|n| n == name), "missing builtin {name}");
    }
    assert!(runtime.lsp_restart("ws-x", "rust-analyzer").await.is_err());
}
