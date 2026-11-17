//! Optional local, content-free timing for logical model calls.
use crate::model::{DeltaSink, ModelAnswer, StreamDelta};
use harness_contracts::AppResult;
use serde_json::{Map, Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static WRITE_LOCK: Mutex<()> = Mutex::new(());

struct RequestState {
    path: PathBuf,
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
        Self(path.map(|path| {
            Arc::new(RequestState {
                path,
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
            .open(&state.path)
            .and_then(|mut file| file.write_all(&line));
        if result.is_err() {
            let _ = writeln!(std::io::stderr().lock(), "请求计时诊断文件写入失败");
        }
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
            path,
            session_id: "session".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            source,
            started: Instant::now(),
            started_unix_ms: unix_ms(),
            saw_delta: AtomicBool::new(false),
            first_delta_ms: Mutex::new(None),
        })))
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
}
