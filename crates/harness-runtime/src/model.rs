//! DeepSeek/OpenAI-compatible streaming transport. Request credentials never enter event logs.
use async_trait::async_trait;
use futures::StreamExt;
use harness_contracts::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default, Clone)]
pub struct ModelAnswer {
    pub content: String,
    pub reasoning: String,
    pub calls: Vec<ToolCall>,
    pub usage: Value,
}
#[derive(Debug, Clone)]
pub struct StreamDelta {
    pub text: String,
    pub reasoning: bool,
}
pub type DeltaSink = Arc<dyn Fn(StreamDelta) + Send + Sync>;

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(
        &self,
        messages: Vec<Value>,
        tools: Vec<ToolDefinition>,
        cancel: CancellationToken,
        sink: DeltaSink,
    ) -> AppResult<ModelAnswer>;
}

pub struct HttpModel {
    pub config: ModelConfig,
    pub secret: String,
}
#[async_trait]
impl ModelProvider for HttpModel {
    async fn complete(
        &self,
        messages: Vec<Value>,
        tools: Vec<ToolDefinition>,
        cancel: CancellationToken,
        sink: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(20));
        if let Some(proxy) = &self.config.proxy {
            if !proxy.is_empty() {
                builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(model_error)?);
            }
        }
        let client = builder.build().map_err(model_error)?;
        let mut body = json!({"model":self.config.model,"messages":messages,"stream":true,"stream_options":{"include_usage":true}});
        if !tools.is_empty() {
            body["tools"] = json!(tools.into_iter().map(|t|json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}})).collect::<Vec<_>>());
        }
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut request = client
            .post(url)
            .json(&body)
            .timeout(Duration::from_secs(180));
        if !self.secret.is_empty() {
            request = request.bearer_auth(&self.secret);
        }
        let response = tokio::select! { _=cancel.cancelled()=>return Err(cancelled()), r=request.send()=>r.map_err(model_error)? };
        let status = response.status();
        if !status.is_success() {
            // Provider bodies can echo credentials; only expose the status to the UI/log.
            return Err(AppError::new(
                "MODEL_HTTP",
                format!("模型服务返回 HTTP {status}；请检查模型、凭据和服务地址"),
            ));
        }
        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        while let Some(chunk) = tokio::select! { _=cancel.cancelled()=>return Err(cancelled()), next=stream.next()=>next }
        {
            for delta in decoder.push(&chunk.map_err(model_error)?)? {
                sink(delta);
            }
        }
        decoder.finish()
    }
}
pub fn cancelled() -> AppError {
    AppError::new("CANCELLED", "操作已取消")
}
fn model_error(e: impl std::fmt::Display) -> AppError {
    AppError::new("MODEL_TRANSPORT", e.to_string())
}

#[derive(Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    answer: ModelAnswer,
    calls: BTreeMap<u64, (String, String, String)>,
    finished: bool,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<StreamDelta>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > 4 * 1024 * 1024 {
            return Err(AppError::new("MODEL_LIMIT", "模型单帧超过4MiB"));
        }
        let mut deltas = Vec::new();
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=pos).collect();
            let text = std::str::from_utf8(&line).map_err(model_error)?.trim();
            let Some(data) = text.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                self.finished = true;
                continue;
            }
            if data.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(data).map_err(model_error)?;
            if v.get("error").is_some() {
                return Err(AppError::new("MODEL_STREAM", "模型服务返回流式错误"));
            }
            if !v["usage"].is_null() {
                self.answer.usage = v["usage"].clone();
            }
            for choice in v["choices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["index"].as_u64().unwrap_or(0) == 0)
            {
                let delta = &choice["delta"];
                for (key, reasoning) in [("content", false), ("reasoning_content", true)] {
                    if let Some(text) = delta[key].as_str() {
                        if reasoning {
                            self.answer.reasoning.push_str(text);
                        } else {
                            self.answer.content.push_str(text);
                        }
                        deltas.push(StreamDelta {
                            text: text.into(),
                            reasoning,
                        });
                    }
                }
                for c in delta["tool_calls"].as_array().into_iter().flatten() {
                    let slot = self
                        .calls
                        .entry(c["index"].as_u64().unwrap_or(0))
                        .or_default();
                    if let Some(id) = c["id"].as_str() {
                        slot.0.push_str(id);
                    }
                    if let Some(name) = c["function"]["name"].as_str() {
                        slot.1.push_str(name);
                    }
                    if let Some(args) = c["function"]["arguments"].as_str() {
                        slot.2.push_str(args);
                    }
                }
            }
        }
        Ok(deltas)
    }
    pub fn finish(mut self) -> AppResult<ModelAnswer> {
        if !self.finished {
            return Err(AppError::new(
                "INCOMPLETE_STREAM",
                "模型流中断，未收到结束标记",
            ));
        }
        for (_, (id, name, args)) in self.calls {
            if id.is_empty() || name.is_empty() {
                return Err(AppError::new("MODEL_TOOL", "模型工具调用缺少标识或名称"));
            }
            let arguments = serde_json::from_str(&args)
                .map_err(|_| AppError::new("MODEL_TOOL", "模型工具参数不是完整JSON"))?;
            self.answer.calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
        Ok(self.answer)
    }
}
