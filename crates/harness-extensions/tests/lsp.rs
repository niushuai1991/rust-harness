use futures::{SinkExt, StreamExt};
use harness_contracts::LspServerConfig;
use harness_extensions::lsp::FrameCodec;
use harness_extensions::lsp::{LspManager, file_uri, path_to_uri};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::{
    codec::{FramedRead, FramedWrite},
    sync::CancellationToken,
};

const ROOT: &str = "/work/ws";

fn config(id: &str) -> LspServerConfig {
    LspServerConfig {
        id: id.into(),
        command: "mock-server".into(),
        args: vec![],
        extensions: vec!["rs".into()],
        initialization_options: Value::Null,
    }
}

#[derive(Default)]
struct MockState {
    requests: Vec<Value>,
    notifications: Vec<Value>,
    docs: BTreeMap<String, String>,
    server_request_error_code: Option<i64>,
}
type SharedState = Arc<Mutex<MockState>>;

/// rust-analyzer normalizes Windows drive letters to lowercase when echoing URIs;
/// the mock reproduces that so cache matching must be case-insensitive to pass.
fn lowercase_drive(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("file:///") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return format!(
                "file:///{}{}",
                (bytes[0] as char).to_ascii_lowercase(),
                &rest[1..]
            );
        }
    }
    uri.to_owned()
}

async fn serve_mock(stream: tokio::io::DuplexStream, state: SharedState) {
    let (read, write) = tokio::io::split(stream);
    let mut reader = FramedRead::new(read, FrameCodec::new());
    let mut writer = FramedWrite::new(write, FrameCodec::new());
    while let Some(Ok(message)) = reader.next().await {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned().filter(|v| !v.is_null());
        if let Some(id) = id.filter(|_| !method.is_empty()) {
            state.lock().unwrap().requests.push(message.clone());
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let result = match method {
                "initialize" => json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "documentSymbolProvider": true,
                        "workspaceSymbolProvider": true,
                    },
                    "serverInfo": {"name": "mock", "version": "0"}
                }),
                "test/echo" => params.clone(),
                "test/never" => continue,
                "textDocument/hover" => json!({
                    "contents": {"kind": "markdown", "value": "MOCK HOVER"}
                }),
                "textDocument/definition" => json!({
                    "uri": params["textDocument"]["uri"],
                    "range": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 8}}
                }),
                "textDocument/references" => json!([
                    {"uri": params["textDocument"]["uri"], "range": {
                        "start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 1}}}
                ]),
                "textDocument/documentSymbol" => json!([
                    {"name": "main", "kind": 12, "range": {
                        "start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 1}},
                     "selectionRange": {
                        "start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}}
                ]),
                "workspace/symbol" => json!([
                    {"name": params["query"], "kind": 12, "location": {
                        "uri": file_uri(Path::new(ROOT), "src/main.rs").unwrap(),
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}}}}
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
        } else if !method.is_empty() {
            state.lock().unwrap().notifications.push(message.clone());
            if method == "textDocument/didOpen" || method == "textDocument/didChange" {
                let uri = message["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                let text = if method == "textDocument/didOpen" {
                    message["params"]["textDocument"]["text"]
                        .as_str()
                        .unwrap_or("")
                } else {
                    message["params"]["contentChanges"][0]["text"]
                        .as_str()
                        .unwrap_or("")
                }
                .to_owned();
                let diagnostics = if text.is_empty() {
                    vec![]
                } else {
                    vec![json!({
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                        "severity": 1,
                        "source": "mock",
                        "message": format!("mock diag len={}", text.len())
                    })]
                };
                state.lock().unwrap().docs.insert(uri.clone(), text);
                let _ = writer
                    .send(json!({
                        "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                        "params": {"uri": lowercase_drive(&uri), "diagnostics": diagnostics}
                    }))
                    .await;
            } else if method == "test/serverRequest" {
                let _ = writer
                    .send(json!({
                        "jsonrpc": "2.0", "id": "srv-1",
                        "method": "workspace/configuration", "params": {"items": []}
                    }))
                    .await;
            }
        } else if let Some(code) = message.get("error").and_then(|e| e["code"].as_i64()) {
            // Response to a server-initiated request.
            state.lock().unwrap().server_request_error_code = Some(code);
        }
    }
    let _ = writer.flush().await;
}

async fn connect_mock(
    manager: &LspManager,
    workspace: &str,
) -> (SharedState, tokio::task::JoinHandle<()>) {
    let (client, server) = tokio::io::duplex(65536);
    let state: SharedState = Arc::new(Mutex::new(MockState::default()));
    let task = tokio::spawn(serve_mock(server, state.clone()));
    let (read, write) = tokio::io::split(client);
    manager
        .connect(workspace, &config("mock"), Path::new(ROOT), read, write)
        .await
        .unwrap();
    (state, task)
}

fn doc_uri(relative: &str) -> String {
    file_uri(Path::new(ROOT), relative).unwrap()
}

#[tokio::test]
async fn connect_handshake_and_echo_roundtrip() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    assert_eq!(connection.server_id(), "mock");
    let params = json!({"hello": "world"});
    let result = connection
        .request("test/echo", params.clone(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result, params);
    let entries = manager.status().await;
    assert_eq!(entries.len(), 1);
    assert!(entries[0].running);
    server.abort();
}

#[tokio::test]
async fn sync_document_opens_then_changes_and_pushes_diagnostics() {
    let manager = LspManager::new();
    let (state, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let uri = doc_uri("src/main.rs");
    connection
        .sync_document(&uri, "rust", "fn main() {}")
        .await
        .unwrap();
    connection
        .sync_document(&uri, "rust", "fn main() { changed }")
        .await
        .unwrap();
    connection
        .sync_document(&uri, "rust", "fn main() { changed }")
        .await
        .unwrap();
    // The mock processes frames asynchronously; wait for it to observe the syncs.
    for _ in 0..100 {
        let notifications = state.lock().unwrap().notifications.clone();
        let opens = notifications
            .iter()
            .filter(|n| n["method"] == "textDocument/didOpen")
            .count();
        let changes = notifications
            .iter()
            .filter(|n| n["method"] == "textDocument/didChange")
            .count();
        if opens == 1 && changes == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let notifications = state.lock().unwrap().notifications.clone();
    let opens = notifications
        .iter()
        .filter(|n| n["method"] == "textDocument/didOpen")
        .count();
    let changes = notifications
        .iter()
        .filter(|n| n["method"] == "textDocument/didChange")
        .count();
    assert_eq!(opens, 1, "first sync must open");
    assert_eq!(
        changes, 1,
        "unchanged text must not re-push, changed text must"
    );
    assert_eq!(state.lock().unwrap().docs[&uri], "fn main() { changed }");
    let mut diagnostics = connection.diagnostics(Some(&uri)).await;
    for _ in 0..100 {
        if diagnostics.as_array().is_some_and(|list| !list.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        diagnostics = connection.diagnostics(Some(&uri)).await;
    }
    assert_eq!(diagnostics[0]["message"], "mock diag len=21");
    server.abort();
}

#[tokio::test]
async fn wait_diagnostics_returns_after_push_quiet_window() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let uri = doc_uri("src/main.rs");
    connection
        .sync_document(&uri, "rust", "fn main() {}")
        .await
        .unwrap();
    let started = Instant::now();
    connection
        .wait_diagnostics(
            &uri,
            Duration::from_millis(150),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(150),
        "must honor quiet window"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "must not wait for full cap"
    );
    // A file that was never pushed holds until the cap instead of returning early.
    let started = Instant::now();
    connection
        .wait_diagnostics(
            &doc_uri("src/never_pushed.rs"),
            Duration::from_millis(50),
            Duration::from_millis(300),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "must hold until cap"
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_requests_match_ids() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let mut tasks = Vec::new();
    for index in 0..5 {
        let connection = connection.clone();
        let cancel = CancellationToken::new();
        tasks.push(tokio::spawn(async move {
            connection
                .request("test/echo", json!({"index": index}), &cancel)
                .await
                .unwrap()
        }));
    }
    for (index, task) in tasks.into_iter().enumerate() {
        let result = task.await.unwrap();
        assert_eq!(result["index"], index);
    }
    server.abort();
}

#[tokio::test]
async fn server_request_receives_method_not_found() {
    let manager = LspManager::new();
    let (state, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    connection
        .notify("test/serverRequest", json!({}))
        .await
        .unwrap();
    for _ in 0..50 {
        if state.lock().unwrap().server_request_error_code.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        state.lock().unwrap().server_request_error_code,
        Some(-32601)
    );
    server.abort();
}

#[tokio::test]
async fn unknown_method_gets_error_reply() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let error = connection
        .request("test/missing", json!({}), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, "SERVER");
    server.abort();
}

#[tokio::test]
async fn eof_marks_connection_dead_and_fails_pending() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let pending = {
        let connection = connection.clone();
        let cancel = CancellationToken::new();
        tokio::spawn(async move { connection.request("test/never", json!({}), &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.abort();
    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.code, "CONNECTION_CLOSED");
    for _ in 0..50 {
        if !manager.status().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Dead connections are reported as not running until pruned.
    let status = manager.status().await;
    assert!(status.iter().all(|e| !e.running));
    // Pruning on access removes the dead connection.
    assert!(manager.connection("ws1", "mock").await.is_none());
}

#[tokio::test]
async fn disconnect_removes_connection() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    manager.disconnect("ws1", "mock").await.unwrap();
    assert!(manager.connection("ws1", "mock").await.is_none());
    let error = connection
        .request("test/echo", json!({}), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, "CONNECTION_CLOSED");
    assert!(manager.disconnect("ws1", "mock").await.is_err());
    server.abort();
}

#[tokio::test]
async fn request_cancelled_returns_immediately() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let cancel = CancellationToken::new();
    let task = {
        let connection = connection.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { connection.request("test/never", json!({}), &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    server.abort();
}

#[tokio::test]
async fn request_timeout_returns() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    let error = connection
        .request_with_timeout(
            "test/never",
            json!({}),
            Duration::from_millis(150),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "TIMEOUT");
    server.abort();
}

#[tokio::test]
async fn status_reports_document_and_diagnostic_counts() {
    let manager = LspManager::new();
    let (_, server) = connect_mock(&manager, "ws1").await;
    let connection = manager.connection("ws1", "mock").await.unwrap();
    connection
        .sync_document(&doc_uri("src/main.rs"), "rust", "fn main() {}")
        .await
        .unwrap();
    connection
        .sync_document(&doc_uri("src/lib.rs"), "rust", "fn lib() {}")
        .await
        .unwrap();
    for _ in 0..100 {
        if manager.status().await.iter().all(|e| e.diagnostics == 2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let entries = manager.status().await;
    let entry = entries.iter().find(|e| e.workspace_id == "ws1").unwrap();
    assert_eq!(entry.documents, 2);
    assert_eq!(entry.diagnostics, 2);
    assert_eq!(entry.server_id, "mock");
    server.abort();
}

#[tokio::test]
async fn connections_are_scoped_per_workspace() {
    let manager = LspManager::new();
    let (_, server_a) = connect_mock(&manager, "ws-a").await;
    let (_, server_b) = connect_mock(&manager, "ws-b").await;
    assert_eq!(manager.status().await.len(), 2);
    assert!(manager.connection("ws-a", "mock").await.is_some());
    assert!(manager.connection("ws-c", "mock").await.is_none());
    server_a.abort();
    server_b.abort();
}

#[test]
fn path_to_uri_normalizes_windows_separators_and_spaces() {
    assert_eq!(path_to_uri(Path::new("/work/ws")), "file:///work/ws");
    let uri = path_to_uri(Path::new("D:\\work\\my project"));
    assert_eq!(uri, "file:///D:/work/my%20project");
}

#[test]
fn file_uri_rejects_escape_and_absolute_relative_paths() {
    let root = Path::new(ROOT);
    assert!(file_uri(root, "src/main.rs").is_ok());
    assert!(file_uri(root, "../outside.rs").is_err());
    assert!(file_uri(root, "/absolute.rs").is_err());
    assert!(file_uri(root, "").is_err());
}
