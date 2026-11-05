//! 极简 CDP（Chrome DevTools Protocol）WebSocket 客户端：请求-响应匹配 + 事件广播。
//!
//! 仅封装本项目所需的 Page/Runtime/Input 域；只面向本地 ws:// 明文连接。
use futures::{SinkExt, StreamExt};
use harness_contracts::{AppError, AppResult};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
pub type EventReceiver = broadcast::Receiver<(String, Value)>;

pub struct CdpConnection {
    next_id: AtomicU64,
    sender: mpsc::Sender<String>,
    events: broadcast::Sender<(String, Value)>,
    pending: Pending,
}

impl CdpConnection {
    /// 连接 page 级调试 WebSocket（来自 /json/list 的 webSocketDebuggerUrl）。
    pub async fn connect(ws_url: &str) -> AppResult<Self> {
        let (ws, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| AppError::new("CDP_CONNECT", "连接调试端口超时"))?
        .map_err(|e| AppError::new("CDP_CONNECT", format!("连接调试端口失败：{e}")))?;
        let (sink, stream) = ws.split();
        let (sender, outgoing) = mpsc::channel::<String>(64);
        let (events, _) = broadcast::channel(256);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        // JoinHandle 落盘即 detach：读写任务随连接生命周期自行结束。
        std::mem::drop(tokio::spawn(writer_task(sink, outgoing)));
        std::mem::drop(tokio::spawn(reader_task(
            stream,
            pending.clone(),
            events.clone(),
        )));
        Ok(Self {
            next_id: AtomicU64::new(1),
            sender,
            events,
            pending,
        })
    }

    /// 发送 CDP 命令并等待响应（默认 30 秒超时）。
    pub async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.call_timeout(method, params, Duration::from_secs(30))
            .await
    }

    pub async fn call_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> AppResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let request = serde_json::json!({"id": id, "method": method, "params": params}).to_string();
        if self.sender.send(request).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(AppError::new("CDP_DISCONNECTED", "调试连接已断开"));
        }
        let payload = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(payload)) => payload,
            Ok(Err(_)) | Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(AppError::new(
                    "CDP_TIMEOUT",
                    format!("{method} 等待响应超时或连接已断开"),
                ));
            }
        };
        payload.map_err(|message| AppError::new("CDP_ERROR", message))
    }

    /// 订阅事件流；应先于触发事件的动作（如导航）调用，避免丢失 loadEventFired。
    pub fn subscribe(&self) -> EventReceiver {
        self.events.subscribe()
    }
}

async fn writer_task<S>(mut sink: S, mut outgoing: mpsc::Receiver<String>)
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    while let Some(text) = outgoing.recv().await {
        if sink.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
}

async fn reader_task<S>(mut stream: S, pending: Pending, events: broadcast::Sender<(String, Value)>)
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut buffer = String::new();
    loop {
        buffer.clear();
        match stream.next().await {
            Some(Ok(Message::Text(text))) => buffer.push_str(text.as_str()),
            Some(Ok(Message::Binary(data))) => buffer.push_str(&String::from_utf8_lossy(&data)),
            Some(Ok(_)) => continue,
            _ => break,
        }
        let Ok(value) = serde_json::from_str::<Value>(&buffer) else {
            continue;
        };
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let waiter = pending.lock().unwrap().remove(&id);
            if let Some(waiter) = waiter {
                let payload = if let Some(result) = value.get("result") {
                    Ok(result.clone())
                } else if let Some(error) = value.get("error") {
                    Err(error["message"]
                        .as_str()
                        .unwrap_or("CDP 调用失败")
                        .to_string())
                } else {
                    Ok(Value::Null)
                };
                let _ = waiter.send(payload);
            }
        } else if let Some(method) = value.get("method").and_then(Value::as_str) {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = events.send((method.to_string(), params));
        }
    }
    // 连接断开：唤醒所有等待者，避免调用悬挂。
    for (_, waiter) in pending.lock().unwrap().drain() {
        let _ = waiter.send(Err("调试连接已断开".into()));
    }
}

/// 在事件流上等待下一匹配事件；超时返回 `false`（不视为错误）。
pub async fn wait_for(receiver: &mut EventReceiver, method: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, receiver.recv()).await {
            Ok(Ok((name, _))) => {
                if name == method {
                    return true;
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => return false,
            Err(_) => return false,
        }
    }
}
