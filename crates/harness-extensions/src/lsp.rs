//! LSP client adapter over sandboxed stdio pipes.
//! Stdio process creation belongs to the platform sandbox; this module accepts its pipes.
use crate::manifest::identifier;
use bytes::Buf;
use futures::{SinkExt, StreamExt};
use harness_contracts::{AppError, AppResult, LspServerConfig, LspStatusEntry};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{RwLock, mpsc, oneshot},
};
use tokio_util::{
    codec::{Decoder, Encoder, FramedRead, FramedWrite},
    sync::CancellationToken,
};

/// Single request timeout; sits inside the runtime's global tool timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// initialize handshake timeout.
pub const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
/// Maximum accepted header block size (LSP base protocol headers are tiny).
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Maximum accepted body size; protects against runaway server output.
pub const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub struct FrameCodec;
impl FrameCodec {
    pub fn new() -> Self {
        Self
    }
}
impl Decoder for FrameCodec {
    type Item = serde_json::Value;
    type Error = std::io::Error;
    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(head) = find_header_end(src) else {
            // Until the terminator arrives every buffered byte is header bytes.
            if src.len() > MAX_HEADER_BYTES {
                return Err(too_large("LSP header block"));
            }
            return Ok(None);
        };
        let headers = std::str::from_utf8(&src[..head]).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "LSP header not UTF-8")
        })?;
        let length = content_length(headers)?;
        if length > MAX_BODY_BYTES {
            return Err(too_large("LSP body"));
        }
        let body_start = head + 4;
        if src.len() < body_start + length {
            return Ok(None);
        }
        let value = serde_json::from_slice(&src[body_start..body_start + length]).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("LSP body: {e}"))
        })?;
        src.advance(body_start + length);
        Ok(Some(value))
    }
}
impl Encoder<Value> for FrameCodec {
    type Error = std::io::Error;
    fn encode(&mut self, item: Value, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        let body = serde_json::to_vec(&item).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("LSP encode: {e}"))
        })?;
        dst.reserve(body.len() + 32);
        dst.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        dst.extend_from_slice(&body);
        Ok(())
    }
}
fn too_large(what: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{what} exceeds limit"),
    )
}
fn find_header_end(src: &[u8]) -> Option<usize> {
    src.windows(4).position(|w| w == b"\r\n\r\n")
}
fn content_length(headers: &str) -> Result<usize, std::io::Error> {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "LSP header missing content-length",
            )
        })
}

fn fail(message: impl ToString) -> AppError {
    AppError::new("LSP", message.to_string())
}
fn closed() -> AppError {
    AppError::new("CONNECTION_CLOSED", "LSP server connection closed")
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
/// Servers normalize Windows drive letters differently from us (rust-analyzer echoes
/// `file:///c:/...` for our `file:///C:/...`); match file URIs case-insensitively on
/// the drive letter so pushed diagnostics align with the URIs we sent.
fn uri_key(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("file:///") {
        let mut chars = rest.chars();
        if let (Some(letter), Some(':')) = (chars.next(), chars.next())
            && letter.is_ascii_alphabetic()
        {
            return format!("file:///{}{}", letter.to_ascii_lowercase(), &rest[1..]);
        }
    }
    uri.to_owned()
}

/// Percent-encode illegal URI path bytes; backslashes become forward slashes first.
fn encode_path(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b'~' | b'-' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
/// Absolute path to a `file://` URI, normalizing Windows separators.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let absolute = if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    };
    format!("file://{}", encode_path(&absolute))
}
/// Workspace-relative path to a `file://` URI; rejects escapes and absolute paths.
pub fn file_uri(root: &Path, relative: &str) -> AppResult<String> {
    if relative.is_empty() {
        return Err(fail("empty relative path"));
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(fail("relative path required"));
    }
    for component in relative_path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(fail(
                "path components outside the workspace are not addressable",
            ));
        }
    }
    Ok(path_to_uri(&root.join(relative_path)))
}
/// Best-effort LSP languageId for a file extension.
pub fn language_for_extension(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "jsx" => "javascriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "cs" => "csharp",
        _ => "plaintext",
    }
}

#[derive(Clone, Copy)]
struct Document {
    version: i32,
    hash: u64,
}

pub struct LspConnection {
    server_id: String,
    next_id: AtomicI64,
    dead: CancellationToken,
    pending: Mutex<BTreeMap<i64, oneshot::Sender<Result<Value, Value>>>>,
    diagnostics: Mutex<BTreeMap<String, Vec<Value>>>,
    push_times: Mutex<BTreeMap<String, Instant>>,
    documents: Mutex<BTreeMap<String, Document>>,
    sync_lock: tokio::sync::Mutex<()>,
    send: mpsc::UnboundedSender<Value>,
}

impl Drop for LspConnection {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl LspConnection {
    fn start<R, W>(server_id: String, read: R, write: W) -> AppResult<Arc<Self>>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (send, receive) = mpsc::unbounded_channel();
        let connection = Arc::new(Self {
            server_id,
            next_id: AtomicI64::new(0),
            dead: CancellationToken::new(),
            pending: Mutex::new(BTreeMap::new()),
            diagnostics: Mutex::new(BTreeMap::new()),
            push_times: Mutex::new(BTreeMap::new()),
            documents: Mutex::new(BTreeMap::new()),
            sync_lock: tokio::sync::Mutex::new(()),
            send,
        });
        Self::spawn_writer(write, receive, connection.dead.clone());
        Self::spawn_reader(read, connection.clone());
        Ok(connection)
    }

    fn spawn_writer<W>(
        write: W,
        mut receive: mpsc::UnboundedReceiver<Value>,
        dead: CancellationToken,
    ) where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut writer = FramedWrite::new(write, FrameCodec::new());
            loop {
                tokio::select! {
                    _ = dead.cancelled() => break,
                    item = receive.recv() => match item {
                        Some(value) => {
                            if writer.send(value).await.is_err() {
                                dead.cancel();
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        });
    }

    fn spawn_reader<R>(read: R, connection: Arc<Self>)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut reader = FramedRead::new(read, FrameCodec::new());
            loop {
                let message = tokio::select! {
                    _ = connection.dead.cancelled() => break,
                    item = reader.next() => match item {
                        Some(Ok(value)) => value,
                        _ => break,
                    }
                };
                connection.handle(message);
            }
            connection.shutdown();
        });
    }

    fn handle(&self, message: Value) {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = message.get("id").cloned().filter(|i| !i.is_null());
        match (id, method) {
            (Some(id), Some(_)) => {
                // Server-initiated requests get a legal MethodNotFound reply so
                // servers never block on a client capability we do not implement.
                let _ = self.send.send(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": "MethodNotFound"}
                }));
            }
            (Some(id), None) => {
                if let Some(numeric) = id.as_i64() {
                    let sender = self.pending.lock().unwrap().remove(&numeric);
                    if let Some(sender) = sender {
                        if let Some(error) = message.get("error") {
                            let _ = sender.send(Err(error.clone()));
                        } else {
                            let _ = sender
                                .send(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
                        }
                    }
                }
            }
            (None, Some(method)) => {
                if method == "textDocument/publishDiagnostics" {
                    let uri = message["params"]["uri"].as_str().unwrap_or("").to_owned();
                    let list = message["params"]["diagnostics"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    self.diagnostics.lock().unwrap().insert(uri_key(&uri), list);
                    self.push_times
                        .lock()
                        .unwrap()
                        .insert(uri_key(&uri), Instant::now());
                }
            }
            (None, None) => {}
        }
    }

    fn shutdown(&self) {
        self.dead.cancel();
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for (_, sender) in pending {
            let _ = sender.send(Err(json!({"code": -32000, "message": "connection closed"})));
        }
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }
    pub fn is_dead(&self) -> bool {
        self.dead.is_cancelled()
    }
    pub(crate) fn document_count(&self) -> usize {
        self.documents.lock().unwrap().len()
    }
    pub(crate) fn diagnostic_count(&self) -> usize {
        self.diagnostics
            .lock()
            .unwrap()
            .values()
            .map(Vec::len)
            .sum()
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> AppResult<Value> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT, cancel)
            .await
    }
    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> AppResult<Value> {
        if self.dead.is_cancelled() {
            return Err(closed());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);
        if self
            .send
            .send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .is_err()
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(closed());
        }
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AppError::new("CANCELLED", "LSP request cancelled")),
            _ = tokio::time::sleep(timeout) => {
                Err(AppError::new("TIMEOUT", format!("LSP request {method} timed out")))
            }
            response = receiver => match response {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => {
                    if self.dead.is_cancelled() {
                        Err(closed())
                    } else {
                        Err(AppError::new("SERVER", format!("LSP server error: {error}")))
                    }
                }
                Err(_) => Err(closed()),
            }
        };
        self.pending.lock().unwrap().remove(&id);
        result
    }

    pub async fn notify(&self, method: &str, params: Value) -> AppResult<()> {
        if self.dead.is_cancelled() {
            return Err(closed());
        }
        self.send
            .send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .map_err(|_| closed())
    }

    /// didOpen on first sight, full-text didChange when content differs, no-op when unchanged.
    pub async fn sync_document(&self, uri: &str, language_id: &str, text: &str) -> AppResult<()> {
        if self.dead.is_cancelled() {
            return Err(closed());
        }
        let _guard = self.sync_lock.lock().await;
        let hash = hash_text(text);
        let action = {
            let mut documents = self.documents.lock().unwrap();
            match documents.get(uri) {
                Some(document) if document.hash == hash => None,
                Some(document) => {
                    let version = document.version + 1;
                    documents.insert(uri.to_owned(), Document { version, hash });
                    Some((version, false))
                }
                None => {
                    documents.insert(uri.to_owned(), Document { version: 0, hash });
                    Some((0, true))
                }
            }
        };
        match action {
            None => Ok(()),
            Some((version, open)) => {
                if open {
                    self.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": uri, "languageId": language_id, "version": version, "text": text}}),
                    )
                    .await
                } else {
                    self.notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": {"uri": uri, "version": version},
                            "contentChanges": [{"text": text}]
                        }),
                    )
                    .await
                }
            }
        }
    }

    /// Wait for this file's diagnostics: after the first push for `uri`, return once
    /// the push stream stays quiet for `idle`; if the file was never pushed, hold until
    /// the first push or `cap`. Covers language servers whose cold-start analysis lags.
    pub async fn wait_diagnostics(
        &self,
        uri: &str,
        idle: Duration,
        cap: Duration,
        cancel: &CancellationToken,
    ) -> AppResult<()> {
        let started = Instant::now();
        loop {
            let pushed = self.push_times.lock().unwrap().get(&uri_key(uri)).copied();
            if pushed.is_some_and(|reference| reference.elapsed() >= idle) {
                return Ok(());
            }
            if started.elapsed() >= cap {
                return Ok(());
            }
            // Re-check push arrival in bounded slices instead of sleeping the whole cap.
            let remaining = (match pushed {
                Some(reference) => idle.saturating_sub(reference.elapsed()),
                None => cap.saturating_sub(started.elapsed()),
            })
            .min(Duration::from_millis(50));
            tokio::select! {
                _ = cancel.cancelled() => return Err(AppError::new("CANCELLED", "LSP diagnostics wait cancelled")),
                _ = tokio::time::sleep(remaining) => {}
            }
        }
    }

    /// Cached diagnostics: one file or the whole workspace.
    pub async fn diagnostics(&self, uri: Option<&str>) -> Value {
        let diagnostics = self.diagnostics.lock().unwrap();
        match uri {
            Some(uri) => Value::Array(diagnostics.get(&uri_key(uri)).cloned().unwrap_or_default()),
            None => Value::Array(
                diagnostics
                    .iter()
                    .map(|(uri, list)| json!({"uri": uri, "diagnostics": list}))
                    .collect(),
            ),
        }
    }
}

#[derive(Default)]
pub struct LspManager {
    servers: RwLock<BTreeMap<(String, String), Arc<LspConnection>>>,
}

impl LspManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The caller must create both pipes via the platform sandbox; the config comes
    /// from user settings, which is the authorization for the child process.
    pub async fn connect<R, W>(
        &self,
        workspace_id: &str,
        server: &LspServerConfig,
        root: &Path,
        read: R,
        write: W,
    ) -> AppResult<Arc<LspConnection>>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        if !identifier(&server.id) {
            return Err(fail("invalid LSP server id"));
        }
        if !identifier(workspace_id) {
            return Err(fail("invalid workspace id"));
        }
        {
            let servers = self.servers.read().await;
            if servers
                .get(&(workspace_id.to_owned(), server.id.clone()))
                .is_some_and(|existing| !existing.is_dead())
            {
                return Err(fail("LSP server already running"));
            }
        }
        let connection = LspConnection::start(server.id.clone(), read, write)?;
        let mut params = json!({
            "processId": Value::Null,
            "rootUri": path_to_uri(root),
            "capabilities": {},
            "workspaceFolders": [{
                "uri": path_to_uri(root),
                "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace")
            }]
        });
        if !server.initialization_options.is_null() {
            params["initializationOptions"] = server.initialization_options.clone();
        }
        let result = connection
            .request_with_timeout(
                "initialize",
                params,
                INITIALIZE_TIMEOUT,
                &CancellationToken::new(),
            )
            .await;
        if let Err(error) = result {
            connection.shutdown();
            return Err(fail(format!("LSP initialize failed: {error}")));
        }
        if let Err(error) = connection.notify("initialized", json!({})).await {
            connection.shutdown();
            return Err(fail(format!(
                "LSP initialized notification failed: {error}"
            )));
        }
        self.servers.write().await.insert(
            (workspace_id.to_owned(), server.id.clone()),
            connection.clone(),
        );
        Ok(connection)
    }

    /// Running connection for this workspace, pruning dead entries on access.
    pub async fn connection(
        &self,
        workspace_id: &str,
        server_id: &str,
    ) -> Option<Arc<LspConnection>> {
        let mut servers = self.servers.write().await;
        let key = (workspace_id.to_owned(), server_id.to_owned());
        if servers.get(&key).is_some_and(|c| c.is_dead()) {
            servers.remove(&key);
            return None;
        }
        servers.get(&key).cloned()
    }

    pub async fn disconnect(&self, workspace_id: &str, server_id: &str) -> AppResult<()> {
        let mut servers = self.servers.write().await;
        let key = (workspace_id.to_owned(), server_id.to_owned());
        let connection = servers
            .remove(&key)
            .ok_or_else(|| fail("LSP server not connected"))?;
        connection.shutdown();
        Ok(())
    }

    pub async fn status(&self) -> Vec<LspStatusEntry> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|((workspace_id, server_id), connection)| LspStatusEntry {
                workspace_id: workspace_id.clone(),
                server_id: server_id.clone(),
                running: !connection.is_dead(),
                documents: connection.document_count() as u32,
                diagnostics: connection.diagnostic_count() as u32,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_keys_match_drive_letters_case_insensitively() {
        assert_eq!(uri_key("file:///C:/work/a.rs"), "file:///c:/work/a.rs");
        assert_eq!(uri_key("file:///c:/work/a.rs"), "file:///c:/work/a.rs");
        assert_eq!(uri_key("file:///work/a.rs"), "file:///work/a.rs");
        assert_eq!(uri_key("untitled:Untitled-1"), "untitled:Untitled-1");
    }

    #[test]
    fn language_ids_cover_common_extensions() {
        assert_eq!(language_for_extension("rs"), "rust");
        assert_eq!(language_for_extension("TS"), "typescript");
        assert_eq!(language_for_extension("py"), "python");
        assert_eq!(language_for_extension("unknown"), "plaintext");
    }

    #[test]
    fn encode_path_keeps_unreserved_and_colon() {
        assert_eq!(encode_path("/D:/work/a b#frag"), "/D:/work/a%20b%23frag");
    }
}
