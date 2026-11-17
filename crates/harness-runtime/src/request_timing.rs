//! Optional local, content-free timing for logical model calls.
use crate::model::{DeltaSink, ModelAnswer, StreamDelta};
use base64::{Engine, engine::general_purpose::STANDARD};
use harness_contracts::AppResult;
use serde_json::{Map, Value, json};
use std::{
    fs::OpenOptions,
    future::Future,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static WRITE_LOCK: Mutex<()> = Mutex::new(());
const SSE_CAPTURE_LIMIT: usize = 8 * 1024 * 1024;

tokio::task_local! {
    static REQUEST_CONTEXT: Arc<RequestState>;
}

struct RequestState {
    path: Option<PathBuf>,
    sse_path: Option<PathBuf>,
    session_id: String,
    request_id: String,
    source: &'static str,
    started: Instant,
    started_unix_ms: u128,
    saw_delta: AtomicBool,
    first_delta_ms: Mutex<Option<f64>>,
}

/// Emits one JSON line on completion, or when the future is dropped.
pub struct RequestTiming(Option<Arc<RequestState>>);

impl RequestTiming {
    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    pub fn start(session_id: &str, source: &'static str) -> Self {
        let path = std::env::var_os("RUST_HARNESS_REQUEST_TRACE_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let sse_path = std::env::var_os("RUST_HARNESS_SSE_TRACE_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self((path.is_some() || sse_path.is_some()).then(|| {
            Arc::new(RequestState {
                path,
                sse_path,
                session_id: session_id.to_owned(),
                request_id: uuid::Uuid::new_v4().to_string(),
                source,
                started: Instant::now(),
                started_unix_ms: unix_ms(),
                saw_delta: AtomicBool::new(false),
                first_delta_ms: Mutex::new(None),
            })
        }))
    }

    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        if let Some(state) = &self.0 {
            REQUEST_CONTEXT.scope(state.clone(), future).await
        } else {
            future.await
        }
    }

    pub fn wrap_sink(&self, sink: DeltaSink) -> DeltaSink {
        let Some(state) = self.0.clone() else {
            return sink;
        };
        Arc::new(move |delta: StreamDelta| {
            if !delta.text.is_empty() && !state.saw_delta.swap(true, Ordering::Relaxed) {
                let mut first = state
                    .first_delta_ms
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *first = Some(state.started.elapsed().as_secs_f64() * 1000.0);
            }
            sink(delta);
        })
    }

    pub fn finish(&mut self, result: &AppResult<ModelAnswer>, partial_usage: Option<&Value>) {
        let (status, usage) = match result {
            Ok(answer) => ("completed", Some(&answer.usage)),
            Err(error) if error.code == "CANCELLED" => ("cancelled", partial_usage),
            Err(_) => ("error", partial_usage),
        };
        self.write(status, usage);
    }

    fn write(&mut self, status: &str, usage: Option<&Value>) {
        let Some(state) = self.0.take() else { return };
        let Some(path) = &state.path else { return };
        let first_delta_ms = *state
            .first_delta_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let record = json!({
            "session_id": state.session_id,
            "request_id": state.request_id,
            "source": state.source,
            "status": status,
            "started_unix_ms": state.started_unix_ms,
            "finished_unix_ms": unix_ms(),
            "duration_ms": state.started.elapsed().as_secs_f64() * 1000.0,
            "first_delta_ms": first_delta_ms,
            "usage": usage.map(safe_usage),
        });
        let Ok(mut line) = serde_json::to_vec(&record) else {
            return;
        };
        line.push(b'\n');
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(&line));
        if result.is_err() {
            let _ = writeln!(std::io::stderr().lock(), "请求计时诊断文件写入失败");
        }
    }
}

/// Optional bounded copy of bytes delivered to the SSE decoder.
pub(crate) struct SseCapture {
    state: Option<Arc<RequestState>>,
    bytes: Vec<u8>,
    received_bytes: u64,
    started_unix_ms: u128,
    http_status: Option<u16>,
    observed_usage: Option<Value>,
}

impl SseCapture {
    pub(crate) fn start() -> Self {
        let state = REQUEST_CONTEXT
            .try_with(|state| state.sse_path.as_ref().map(|_| state.clone()))
            .ok()
            .flatten();
        Self {
            state,
            bytes: Vec::new(),
            received_bytes: 0,
            started_unix_ms: unix_ms(),
            http_status: None,
            observed_usage: None,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.state.is_some()
    }

    pub(crate) fn observe_usage(&mut self, usage: &Value) {
        let filtered = safe_usage(usage);
        if filtered.as_object().is_some_and(|v| !v.is_empty()) {
            self.observed_usage = Some(filtered);
        }
    }

    pub(crate) fn set_http_status(&mut self, status: u16) {
        self.http_status = Some(status);
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.state.is_none() {
            return;
        }
        self.received_bytes = self.received_bytes.saturating_add(bytes.len() as u64);
        let available = SSE_CAPTURE_LIMIT.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&bytes[..bytes.len().min(available)]);
    }

    pub(crate) fn finish(&mut self, result: &AppResult<ModelAnswer>) {
        let (status, usage) = match result {
            Ok(answer) => ("completed", Some(safe_usage(&answer.usage))),
            Err(error) if error.code == "CANCELLED" => ("cancelled", self.observed_usage.take()),
            Err(_) => ("error", self.observed_usage.take()),
        };
        self.write(status, usage);
    }

    fn write(&mut self, status: &str, usage: Option<Value>) {
        let Some(state) = self.state.take() else {
            return;
        };
        let Some(path) = &state.sse_path else { return };
        let record = json!({
            "session_id": state.session_id,
            "request_id": state.request_id,
            "source": state.source,
            "status": status,
            "http_status": self.http_status,
            "started_unix_ms": self.started_unix_ms,
            "finished_unix_ms": unix_ms(),
            "received_bytes": self.received_bytes,
            "captured_bytes": self.bytes.len(),
            "truncated": self.received_bytes > self.bytes.len() as u64,
            "sse_base64": STANDARD.encode(&self.bytes),
            "usage": usage,
        });
        let Ok(mut line) = serde_json::to_vec(&record) else {
            return;
        };
        line.push(b'\n');
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(&line))
            .is_err()
        {
            let _ = writeln!(std::io::stderr().lock(), "响应流诊断文件写入失败");
        }
    }
}

impl Drop for SseCapture {
    fn drop(&mut self) {
        let usage = self.observed_usage.take();
        self.write("dropped", usage);
    }
}

impl Drop for RequestTiming {
    fn drop(&mut self) {
        self.write("dropped", None);
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn safe_usage(value: &Value) -> Value {
    let mut safe = Map::new();
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
    ] {
        if let Some(number) = value.get(key).and_then(Value::as_u64) {
            safe.insert(key.into(), json!(number));
        }
    }
    for key in ["prompt_tokens_details", "input_tokens_details"] {
        if let Some(number) = value
            .get(key)
            .and_then(|detail| detail.get("cached_tokens"))
            .and_then(Value::as_u64)
        {
            safe.insert(key.into(), json!({"cached_tokens": number}));
        }
    }
    Value::Object(safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_contracts::AppError;

    fn timer(path: PathBuf, source: &'static str) -> RequestTiming {
        RequestTiming(Some(Arc::new(RequestState {
            path: Some(path),
            sse_path: None,
            session_id: "session".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            source,
            started: Instant::now(),
            started_unix_ms: unix_ms(),
            saw_delta: AtomicBool::new(false),
            first_delta_ms: Mutex::new(None),
        })))
    }

    fn sse_timer(path: PathBuf, source: &'static str) -> RequestTiming {
        let mut timing = timer(path.clone(), source);
        let state = Arc::get_mut(timing.0.as_mut().unwrap()).unwrap();
        state.path = None;
        state.sse_path = Some(path);
        timing
    }

    fn read(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn ignores_empty_delta_and_filters_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        let mut timing = timer(path.clone(), "session_title");
        let sink = timing.wrap_sink(Arc::new(|_| {}));
        sink(StreamDelta {
            text: String::new(),
            reasoning: false,
        });
        let answer = ModelAnswer {
            usage: json!({"input_tokens": 4, "cache_read_input_tokens": 3, "prompt_tokens_details": {"cached_tokens": 2, "secret": "PRIVATE"}, "input_tokens_details": {"cached_tokens": 1, "secret": "PRIVATE"}, "secret": "PRIVATE"}),
            ..Default::default()
        };
        timing.finish(&Ok(answer), None);
        let records = read(&path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["first_delta_ms"], Value::Null);
        assert_eq!(
            records[0]["usage"],
            json!({"input_tokens": 4, "cache_read_input_tokens": 3, "prompt_tokens_details": {"cached_tokens": 2}, "input_tokens_details": {"cached_tokens": 1}})
        );
        assert!(!std::fs::read_to_string(path).unwrap().contains("PRIVATE"));
    }

    #[test]
    fn records_error_cancellation_and_dropped_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        let mut failed = timer(path.clone(), "main_turn");
        failed.finish(
            &Err(AppError::new("MODEL_HTTP", "PRIVATE")),
            Some(&json!({"output_tokens": 9})),
        );
        let mut cancelled = timer(path.clone(), "compact");
        cancelled.finish(&Err(AppError::new("CANCELLED", "PRIVATE")), None);
        drop(timer(path.clone(), "session_title"));
        let records = read(&path);
        assert_eq!(
            records
                .iter()
                .map(|r| r["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["error", "cancelled", "dropped"]
        );
        assert_eq!(records[0]["usage"]["output_tokens"], 9);
        assert!(!std::fs::read_to_string(path).unwrap().contains("PRIVATE"));
    }

    #[test]
    fn concurrent_requests_write_whole_lines_with_distinct_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let path = path.clone();
                scope.spawn(move || drop(timer(path, "main_turn")));
            }
        });
        let records = read(&path);
        assert_eq!(records.len(), 16);
        let ids: std::collections::HashSet<_> = records
            .iter()
            .map(|r| r["request_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 16);
    }

    #[test]
    fn unwritable_diagnostic_destination_does_not_fail_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut timing = timer(dir.path().to_path_buf(), "main_turn");
        let result = Ok(ModelAnswer::default());
        timing.finish(&result, None);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn sse_only_preserves_bytes_and_safe_usage_without_timing_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        let timing = sse_timer(path.clone(), "session_title");
        timing.scope(async {
            let mut capture = SseCapture::start();
            capture.set_http_status(200);
            capture.push(b"event: message\r\ndata: ");
            capture.push(&[0xff, b'\n', b'\n']);
            capture.finish(&Ok(ModelAnswer {
                usage: json!({"input_tokens": 5, "cache_read_input_tokens": 3, "secret": "PRIVATE"}),
                ..Default::default()
            }));
        }).await;
        let rows = read(&path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["source"], "session_title");
        assert_eq!(rows[0]["http_status"], 200);
        assert_eq!(rows[0]["status"], "completed");
        assert_eq!(
            rows[0]["usage"],
            json!({"input_tokens": 5, "cache_read_input_tokens": 3})
        );
        assert_eq!(
            STANDARD
                .decode(rows[0]["sse_base64"].as_str().unwrap())
                .unwrap(),
            b"event: message\r\ndata: \xff\n\n"
        );
        assert!(!std::fs::read_to_string(path).unwrap().contains("PRIVATE"));
    }

    #[tokio::test]
    async fn sse_capture_marks_truncation_and_retains_exact_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        let timing = sse_timer(path.clone(), "main_turn");
        timing
            .scope(async {
                let mut capture = SseCapture::start();
                capture.push(&vec![b'a'; SSE_CAPTURE_LIMIT + 7]);
                capture.finish(&Ok(ModelAnswer::default()));
            })
            .await;
        let rows = read(&path);
        assert_eq!(rows[0]["truncated"], true);
        assert_eq!(rows[0]["received_bytes"], SSE_CAPTURE_LIMIT + 7);
        assert_eq!(rows[0]["captured_bytes"], SSE_CAPTURE_LIMIT);
        assert_eq!(
            STANDARD
                .decode(rows[0]["sse_base64"].as_str().unwrap())
                .unwrap(),
            vec![b'a'; SSE_CAPTURE_LIMIT]
        );
    }

    #[tokio::test]
    async fn invalid_sse_destination_does_not_change_model_result() {
        let dir = tempfile::tempdir().unwrap();
        let timing = sse_timer(dir.path().to_path_buf(), "main_turn");
        let result = timing
            .scope(async {
                let mut capture = SseCapture::start();
                capture.push(b"data");
                let result = Ok(ModelAnswer {
                    content: "ok".into(),
                    ..Default::default()
                });
                capture.finish(&result);
                result
            })
            .await;
        assert_eq!(result.unwrap().content, "ok");
    }

    #[tokio::test]
    async fn sse_capture_records_error_cancel_and_dropped_partial_streams() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        for (source, code) in [("main_turn", "MODEL_PROTOCOL"), ("compact", "CANCELLED")] {
            let timing = sse_timer(path.clone(), source);
            timing
                .scope(async {
                    let mut capture = SseCapture::start();
                    capture.push(b"partial");
                    capture
                        .observe_usage(&json!({"input_tokens": 7, "cache_read_input_tokens": 3}));
                    capture.finish(&Err(AppError::new(code, "PRIVATE")));
                })
                .await;
        }
        let timing = sse_timer(path.clone(), "session_title");
        timing
            .scope(async {
                let mut capture = SseCapture::start();
                capture.push(b"partial");
                capture.observe_usage(&json!({"input_tokens": 7, "cache_read_input_tokens": 3}));
            })
            .await;
        let rows = read(&path);
        assert_eq!(
            rows.iter()
                .map(|r| r["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["error", "cancelled", "dropped"]
        );
        for row in rows {
            assert_eq!(row["usage"]["cache_read_input_tokens"], 3);
            assert_eq!(
                STANDARD
                    .decode(row["sse_base64"].as_str().unwrap())
                    .unwrap(),
                b"partial"
            );
        }
        assert!(!std::fs::read_to_string(path).unwrap().contains("PRIVATE"));
    }

    #[tokio::test]
    async fn concurrent_scoped_sse_requests_do_not_mix_identifiers_or_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        let mut tasks = Vec::new();
        for n in 0..8 {
            let timing = sse_timer(path.clone(), "main_turn");
            tasks.push(tokio::spawn(async move {
                timing
                    .scope(async {
                        let mut capture = SseCapture::start();
                        capture.push(&[n]);
                        tokio::task::yield_now().await;
                        capture.finish(&Ok(ModelAnswer::default()));
                    })
                    .await;
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let rows = read(&path);
        assert_eq!(rows.len(), 8);
        let ids: std::collections::HashSet<_> = rows
            .iter()
            .map(|r| r["request_id"].as_str().unwrap())
            .collect();
        let bytes: std::collections::HashSet<_> = rows
            .iter()
            .map(|r| STANDARD.decode(r["sse_base64"].as_str().unwrap()).unwrap()[0])
            .collect();
        assert_eq!(ids.len(), 8);
        assert_eq!(bytes.len(), 8);
    }

    #[tokio::test]
    async fn anthropic_sse_preserves_original_usage_events_and_excludes_auth_headers() {
        use crate::model::{HttpModel, ModelProvider};
        use harness_contracts::ModelConfig;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_util::sync::CancellationToken;

        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":6,\"cache_creation_input_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"ok\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(17) {
                socket.write_all(chunk).await.unwrap();
            }
        });
        let config: ModelConfig = serde_json::from_value(json!({"id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"","providerId":"fixture","apiMode":"anthropic","maxOutputTokens":1024})).unwrap();
        let provider = HttpModel {
            config,
            secret: "fixture-secret".into(),
            extra_headers: Vec::new(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
            connection_reuse: true,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        let timing = sse_timer(path.clone(), "compact");
        let answer = timing
            .scope(provider.complete(vec![], vec![], CancellationToken::new(), Arc::new(|_| {})))
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(answer.usage["cache_read_input_tokens"], 6);
        let rows = read(&path);
        assert_eq!(rows[0]["usage"]["cache_read_input_tokens"], 6);
        assert_eq!(
            STANDARD
                .decode(rows[0]["sse_base64"].as_str().unwrap())
                .unwrap(),
            body.as_bytes()
        );
        assert!(
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("fixture-secret")
        );
    }

    #[tokio::test]
    async fn failed_anthropic_stream_records_observed_partial_usage() {
        use crate::model::{HttpModel, ModelProvider};
        use harness_contracts::ModelConfig;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_util::sync::CancellationToken;

        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":6}}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"unknown\"}}\n\n"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        });
        let config: ModelConfig = serde_json::from_value(json!({"id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"","providerId":"fixture","apiMode":"anthropic","maxOutputTokens":1024})).unwrap();
        let provider = HttpModel {
            config,
            secret: String::new(),
            extra_headers: Vec::new(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
            connection_reuse: true,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sse.jsonl");
        let timing = sse_timer(path.clone(), "main_turn");
        let error = timing
            .scope(provider.complete(vec![], vec![], CancellationToken::new(), Arc::new(|_| {})))
            .await
            .unwrap_err();
        assert_eq!(error.code, "MODEL_PROTOCOL");
        server.await.unwrap();
        let rows = read(&path);
        assert_eq!(rows[0]["status"], "error");
        assert_eq!(rows[0]["usage"]["cache_read_input_tokens"], 6);
    }
}
