//! HTTP 连接复用测试：同配置共享连接池（keep-alive），不同配置隔离，开关关闭回退既有行为。
use harness_contracts::*;
use harness_runtime::model::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

fn sse_answer() -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]})
    ) + &format!(
        "data: {}\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}})
    ) + "data: [DONE]\n\n"
}

/// 极简 HTTP/1.1 keep-alive 服务：记录每个连接承载的请求数。
async fn spawn_keepalive_server(body: String) -> (String, Arc<Mutex<Vec<usize>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let counts = Arc::new(Mutex::new(Vec::new()));
    let server_counts = counts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            {
                let mut counts = server_counts.lock().await;
                counts.push(0);
            }
            let index = server_counts.lock().await.len() - 1;
            for _ in 0..5 {
                // 请求头读取带超时保护：客户端未复用连接时该连接静默挂起，超时关闭。
                let read = tokio::time::timeout(Duration::from_secs(10), async {
                    let mut bytes = Vec::new();
                    loop {
                        let mut buffer = [0u8; 4096];
                        // 客户端关闭连接（每请求新建客户端的正常行为）或读取失败：结束本连接。
                        let Ok(n) = socket.read(&mut buffer).await else {
                            break None;
                        };
                        if n == 0 {
                            break None;
                        }
                        bytes.extend_from_slice(&buffer[..n]);
                        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                            break Some(bytes);
                        }
                    }
                })
                .await;
                let Ok(Some(bytes)) = read else {
                    break;
                };
                let header_end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
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
                let mut body_bytes = bytes;
                while body_bytes.len() < header_end + length {
                    let mut buffer = [0u8; 4096];
                    let Ok(n) = socket.read(&mut buffer).await else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    body_bytes.extend_from_slice(&buffer[..n]);
                }
                server_counts.lock().await[index] += 1;
                // 不发送 Connection: close：保持 keep-alive 让客户端复用。
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(body.as_bytes()).await.unwrap();
            }
        }
    });
    (base, counts)
}

fn model_config(base: &str, auth: ProviderAuth) -> ModelConfig {
    serde_json::from_value::<ModelConfig>(json!({
        "id":"test","name":"test","baseUrl":base,"model":"model","credentialRef":"",
        "proxy":null,"providerId":"fixture","apiMode":"chatCompletions",
        "maxOutputTokens":1024,"auth":auth
    }))
    .unwrap()
}

async fn complete_once(config: &ModelConfig, connection_reuse: bool) {
    let provider = HttpModel {
        config: config.clone(),
        secret: "fixture-secret".into(),
        extra_headers: Vec::new(),
        stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
        connection_reuse,
    };
    let answer = provider
        .complete(vec![], vec![], CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert_eq!(answer.content, "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_config_reuses_connection() {
    let (base, counts) = spawn_keepalive_server(sse_answer()).await;
    let config = model_config(&base, ProviderAuth::ApiKey);
    complete_once(&config, true).await;
    complete_once(&config, true).await;
    let counts = counts.lock().await;
    assert_eq!(counts.len(), 1, "同配置两次请求应共享同一连接池");
    assert_eq!(counts[0], 2, "keep-alive 连接应承载两次请求");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn different_transport_config_uses_isolated_connection() {
    let (base, counts) = spawn_keepalive_server(sse_answer()).await;
    let plain = model_config(&base, ProviderAuth::ApiKey);
    let oauth = model_config(&base, ProviderAuth::ChatGptOAuth);
    complete_once(&plain, true).await;
    complete_once(&oauth, true).await;
    let counts = counts.lock().await;
    assert_eq!(
        counts.len(),
        2,
        "不同传输配置（OAuth User-Agent）不得共享连接池"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reuse_disabled_builds_a_connection_per_call() {
    let (base, counts) = spawn_keepalive_server(sse_answer()).await;
    let config = model_config(&base, ProviderAuth::ApiKey);
    complete_once(&config, false).await;
    complete_once(&config, false).await;
    let counts = counts.lock().await;
    assert_eq!(
        counts.len(),
        2,
        "复用关闭时应保持既有行为（每请求新建客户端）"
    );
    let _ = Value::Null; // serde_json 引用保活
}
