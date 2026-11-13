//! Real local HTTP/SSE fixtures: no provider credentials or external network.
use harness_contracts::*;
use harness_runtime::{Runtime, model::*};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

struct Captured {
    path: String,
    headers: String,
    body: Value,
}
fn sse(v: Value) -> String {
    format!("data: {v}\n\n")
}
fn answer(mode: ApiMode, step: usize) -> String {
    let calls = match step {
        0 => vec![
            json!({"id":"r","name":"read","arguments":{"path":"a.txt"}}),
            json!({"id":"l","name":"list_files","arguments":{}}),
        ],
        1 => vec![json!({"id":"w","name":"write","arguments":{"path":"a.txt","content":"new"}})],
        _ => vec![],
    };
    match mode {
        ApiMode::ChatCompletions => {
            sse(
                json!({"choices":[{"index":0,"delta":{"content":if calls.is_empty(){"finished"}else{""},"tool_calls":calls.iter().enumerate().map(|(i,c)|json!({"index":i,"id":c["id"],"type":"function","function":{"name":c["name"],"arguments":c["arguments"].to_string()}})).collect::<Vec<_>>()},"finish_reason":if calls.is_empty(){"stop"}else{"tool_calls"}}]}),
            ) + &sse(json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2}}))
                + "data: [DONE]\n\n"
        }
        ApiMode::Responses => {
            let mut output = vec![
                json!({"type":"reasoning","id":format!("reason-{step}"),"encrypted_content":"private-reasoning"}),
            ];
            for c in &calls {
                output.push(json!({"type":"function_call","id":format!("item-{}",c["id"].as_str().unwrap()),"call_id":c["id"],"name":c["name"],"arguments":c["arguments"].to_string()}));
            }
            if calls.is_empty() {
                output.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"finished","annotations":[]}]}));
            }
            let mut stream = if calls.is_empty() {
                sse(json!({"type":"response.output_text.delta","delta":"finished"}))
            } else {
                String::new()
            };
            stream += &sse(
                json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":10,"output_tokens":2}}}),
            );
            stream
        }
        ApiMode::Anthropic => {
            let mut blocks = vec![
                json!({"type":"thinking","thinking":"consider","signature":"private-signature"}),
            ];
            for c in &calls {
                blocks.push(
                    json!({"type":"tool_use","id":c["id"],"name":c["name"],"input":c["arguments"]}),
                );
            }
            if calls.is_empty() {
                blocks.push(json!({"type":"text","text":"finished"}));
            }
            let mut stream =
                sse(json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}));
            for (i, block) in blocks.into_iter().enumerate() {
                stream +=
                    &sse(json!({"type":"content_block_start","index":i,"content_block":block}));
                stream += &sse(json!({"type":"content_block_stop","index":i}));
            }
            stream += &sse(
                json!({"type":"message_delta","delta":{"stop_reason":if calls.is_empty(){"end_turn"}else{"tool_use"}},"usage":{"output_tokens":2}}),
            );
            stream += &sse(json!({"type":"message_stop"}));
            stream
        }
    }
}
async fn fixture(
    mode: ApiMode,
    count: usize,
) -> (
    String,
    mpsc::Receiver<Captured>,
    tokio::task::JoinHandle<()>,
) {
    fixture_bodies((0..count).map(|step| answer(mode, step)).collect()).await
}
async fn fixture_bodies(
    bodies: Vec<String>,
) -> (
    String,
    mpsc::Receiver<Captured>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/prefix", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel(bodies.len());
    let job = tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
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
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            tx.send(Captured {
                path: headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .into(),
                headers: headers.to_lowercase(),
                body: serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
            })
            .await
            .unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(17) {
                socket.write_all(chunk).await.unwrap();
            }
        }
    });
    (base, rx, job)
}

#[tokio::test]
async fn malformed_tool_arguments_still_report_completed_usage() {
    let malformed = sse(json!({
        "choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"bad","type":"function","function":{"name":"todo_write","arguments":"{"}}]},"finish_reason":"tool_calls"}]
    })) + &sse(
        json!({"choices":[],"usage":{"prompt_tokens":4,"completion_tokens":1}}),
    ) + "data: [DONE]\n\n";
    let (base, mut requests, server) = fixture_bodies(vec![malformed]).await;
    let config: ModelConfig = serde_json::from_value(json!({"id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"","proxy":null,"providerId":"fixture","apiMode":"chatCompletions","maxOutputTokens":1024})).unwrap();
    let provider = HttpModel {
        extra_headers: Vec::new(),
        config,
        secret: "fixture-secret".into(),
        stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
    };
    let observed = Arc::new(Mutex::new(None));
    let observed_by_sink = observed.clone();
    let error = provider
        .complete_with_partial_usage(
            vec![],
            vec![],
            CancellationToken::new(),
            Arc::new(|_| {}),
            Arc::new(move |usage| *observed_by_sink.lock().unwrap() = Some(usage)),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "MODEL_PROTOCOL");
    assert_eq!(
        observed.lock().unwrap().as_ref().unwrap(),
        &json!({"prompt_tokens":4,"completion_tokens":1})
    );
    let _request = requests.recv().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn responses_eof_frame_usage_survives_tool_argument_decode_error() {
    let terminal = json!({
        "type":"response.completed",
        "response":{
            "status":"completed",
            "output":[{"type":"function_call","call_id":"bad","name":"todo_write","arguments":"{"}],
            "usage":{"input_tokens":4,"output_tokens":1}
        }
    });
    // Leave the terminal event in the decoder's EOF buffer to exercise finish-time usage capture.
    let (base, mut requests, server) = fixture_bodies(vec![format!("data: {terminal}")]).await;
    let config: ModelConfig = serde_json::from_value(json!({"id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"","proxy":null,"providerId":"fixture","apiMode":"responses","maxOutputTokens":1024})).unwrap();
    let provider = HttpModel {
        extra_headers: Vec::new(),
        config,
        secret: "fixture-secret".into(),
        stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
    };
    let observed = Arc::new(Mutex::new(None));
    let observed_by_sink = observed.clone();
    let error = provider
        .complete_with_partial_usage(
            vec![],
            vec![],
            CancellationToken::new(),
            Arc::new(|_| {}),
            Arc::new(move |usage| *observed_by_sink.lock().unwrap() = Some(usage)),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "INCOMPLETE_STREAM");
    assert_eq!(
        observed.lock().unwrap().as_ref().unwrap(),
        &json!({"input_tokens":4,"output_tokens":1})
    );
    let _request = requests.recv().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn all_protocols_execute_approved_tools_and_replay_after_restart() {
    for mode in [
        ApiMode::ChatCompletions,
        ApiMode::Responses,
        ApiMode::Anthropic,
    ] {
        let (base, mut requests, server) = fixture(mode, 4).await;
        let config:ModelConfig=serde_json::from_value(json!({"id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"","proxy":null,"providerId":"fixture","apiMode":mode,"maxOutputTokens":1024})).unwrap();
        let provider = Arc::new(HttpModel {
            extra_headers: Vec::new(),
            config,
            secret: "fixture-secret".into(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
        });
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a.txt"), "old").unwrap();
        let data = dir.path().join("host");
        let runtime = Runtime::open(data.clone()).await.unwrap();
        let workspace = runtime
            .workspaces
            .create(&source, None, harness_contracts::ExecutionMode::Replica)
            .unwrap();
        let session = runtime
            .store
            .create_session(&workspace.id, "protocol")
            .await
            .unwrap();
        let task = tokio::spawn(runtime.clone().run_session(
            session.id.clone(),
            "change".into(),
            provider.clone(),
        ));
        let approval = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(a) = runtime.approvals.list().into_iter().next() {
                    break a;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(approval.tool, "write");
        runtime.approvals.resolve(&approval.id, true).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("a.txt")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&workspace.replica_path).join("a.txt"))
                .unwrap(),
            "new"
        );
        let events = runtime.store.events(&session.id, 0).await.unwrap();
        assert_eq!(events.iter().filter(|e| e.kind == "tool.result").count(), 3);
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("fixture-secret")
        );
        // request.snapshot 曾在每轮请求前全量落库对话历史，现已移除（膨胀源，payload 无消费方）；
        // 请求路由与模式覆盖由上方 captured HTTP 请求断言承担。
        assert!(events.iter().all(|e| e.kind != "request.snapshot"));
        drop(runtime);
        let runtime = Runtime::open(data).await.unwrap();
        runtime
            .clone()
            .run_session(session.id.clone(), "continue".into(), provider)
            .await
            .unwrap();
        let mut captured = Vec::new();
        for _ in 0..4 {
            captured.push(requests.recv().await.unwrap());
        }
        server.await.unwrap();
        for request in &captured {
            assert_eq!(
                request.path,
                match mode {
                    ApiMode::ChatCompletions => "/prefix/chat/completions",
                    ApiMode::Responses => "/prefix/responses",
                    ApiMode::Anthropic => "/prefix/v1/messages",
                }
            );
            if mode == ApiMode::Anthropic {
                assert!(request.headers.contains("x-api-key: fixture-secret"));
                assert!(request.headers.contains("anthropic-version: 2023-06-01"));
                assert!(!request.headers.contains("authorization:"));
            } else {
                assert!(
                    request
                        .headers
                        .contains("authorization: bearer fixture-secret")
                );
                assert!(!request.headers.contains("x-api-key:"));
            }
        }
        let replay = captured[3].body.to_string();
        match mode {
            ApiMode::ChatCompletions => assert!(replay.contains("tool_call_id")),
            ApiMode::Responses => {
                assert!(replay.contains("private-reasoning"));
                assert!(replay.contains("function_call_output"));
                assert!(captured[3].body.get("previous_response_id").is_none());
            }
            ApiMode::Anthropic => {
                assert!(replay.contains("private-signature"));
                assert!(replay.contains("tool_result"));
            }
        }
    }
}

async fn complete_with_idle(idle: std::time::Duration, base: String) -> AppResult<ModelAnswer> {
    let config: ModelConfig = serde_json::from_value(
        json!({"id":"t","name":"t","baseUrl":base,"model":"m","credentialRef":"","proxy":null}),
    )
    .unwrap();
    HttpModel {
        extra_headers: Vec::new(),
        config,
        secret: String::new(),
        stream_idle_timeout: idle,
    }
    .complete(
        vec![ModelMessage::user("hi")],
        vec![],
        CancellationToken::new(),
        Arc::new(|_| {}),
    )
    .await
}

#[tokio::test]
async fn idle_stream_times_out_when_server_stops_sending() {
    // 响应头已发出、先给一帧随后永久静默且不关连接：空闲超时应在窗口内触发。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = [0; 8192];
        assert!(s.read(&mut b).await.unwrap() > 0);
        let body = sse(json!({"choices":[{"index":0,"delta":{"content":"partial"}}]}));
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len() + 512
        );
        s.write_all(head.as_bytes()).await.unwrap();
        s.write_all(body.as_bytes()).await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = complete_with_idle(std::time::Duration::from_millis(300), base)
        .await
        .unwrap_err();
    assert_eq!(error.code, "MODEL_TIMEOUT");
    assert!(error.message.contains("未收到数据"));
    server.abort();
}

#[tokio::test]
async fn missing_response_headers_times_out() {
    // 收到请求后不回响应头：等待响应阶段也按空闲超时处理。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = [0; 8192];
        assert!(s.read(&mut b).await.unwrap() > 0);
        std::future::pending::<()>().await;
    });
    let error = complete_with_idle(std::time::Duration::from_millis(300), base)
        .await
        .unwrap_err();
    assert_eq!(error.code, "MODEL_TIMEOUT");
    assert!(error.message.contains("未收到数据"));
    server.abort();
}

#[tokio::test]
async fn slow_but_continuous_stream_outlives_idle_window() {
    // 每 200ms 一帧、总时长约 2.2s 超过 1s 的空闲窗口：
    // 只要持续有数据就不应超时（旧的按总时长计时会在此误杀）。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = [0; 8192];
        assert!(s.read(&mut b).await.unwrap() > 0);
        let mut frames: Vec<String> = (0..10)
            .map(|i| sse(json!({"choices":[{"index":0,"delta":{"content":format!("c{i}")}}]})))
            .collect();
        frames.push("data: [DONE]\n\n".into());
        let body = frames.concat();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        s.write_all(head.as_bytes()).await.unwrap();
        for frame in frames {
            s.write_all(frame.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
    let answer = complete_with_idle(std::time::Duration::from_secs(1), base)
        .await
        .unwrap();
    assert_eq!(answer.content, "c0c1c2c3c4c5c6c7c8c9");
    server.await.unwrap();
}

#[tokio::test]
async fn waiting_for_a_stream_can_be_cancelled() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (started, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = [0; 8192];
        assert!(s.read(&mut b).await.unwrap() > 0);
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let config: ModelConfig = serde_json::from_value(
        json!({"id":"t","name":"t","baseUrl":base,"model":"m","credentialRef":"","proxy":null}),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        HttpModel {
            extra_headers: Vec::new(),
            config,
            secret: String::new(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
        }
        .complete(
            vec![ModelMessage::user("hi")],
            vec![],
            token,
            Arc::new(|_| {}),
        )
        .await
    });
    ready.await.unwrap();
    cancel.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    server.abort();
}

#[tokio::test]
async fn provider_errors_are_classified_without_exposing_echoed_keys() {
    for (status, error, expected) in [
        (
            400,
            json!({"code":"model_not_found","message":"echoed-secret"}),
            "MODEL_ID",
        ),
        (
            404,
            json!({"param":"model","message":"echoed-secret"}),
            "MODEL_ID",
        ),
        (401, json!({"message":"echoed-secret"}), "MODEL_AUTH"),
        (404, json!({"message":"echoed-secret"}), "MODEL_ENDPOINT"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 8192];
            assert!(s.read(&mut buffer).await.unwrap() > 0);
            let body = json!({"error":error}).to_string();
            let response = format!(
                "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(response.as_bytes()).await.unwrap();
        });
        let config: ModelConfig = serde_json::from_value(
            json!({"id":"t","name":"t","baseUrl":base,"model":"m","credentialRef":"","proxy":null}),
        )
        .unwrap();
        let error = HttpModel {
            extra_headers: Vec::new(),
            config,
            secret: "echoed-secret".into(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
        }
        .complete(
            vec![ModelMessage::user("hi")],
            vec![],
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.to_string().contains("echoed-secret"));
        server.await.unwrap();
    }
}
