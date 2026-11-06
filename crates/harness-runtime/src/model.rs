//! Protocol-neutral model history and cancellable streaming transports.
mod encoding;
mod streaming;
use async_trait::async_trait;
pub use encoding::encode_request;
use futures::StreamExt;
use harness_contracts::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
pub use streaming::SseDecoder;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    #[default]
    User,
    Assistant,
    Tool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoute {
    pub provider_id: String,
    pub api_mode: ApiMode,
    pub model: String,
    pub base_url: String,
}
impl From<&ModelConfig> for ModelRoute {
    fn from(c: &ModelConfig) -> Self {
        Self {
            provider_id: c.provider_id.clone(),
            api_mode: c.api_mode,
            model: c.model.clone(),
            base_url: c.base_url.trim_end_matches('/').into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolReplay {
    pub route: ModelRoute,
    pub items: Vec<Value>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    pub replay: Option<ProtocolReplay>,
}
impl ModelMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            ..Default::default()
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }
    pub fn tool(id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_call_id: Some(id.into()),
            is_error,
            ..Default::default()
        }
    }
}
#[derive(Debug, Default, Clone)]
pub struct ModelAnswer {
    pub content: String,
    pub reasoning: String,
    pub calls: Vec<ToolCall>,
    pub usage: Value,
    pub replay: Option<ProtocolReplay>,
}
#[derive(Debug, Clone)]
pub struct StreamDelta {
    pub text: String,
    pub reasoning: bool,
}
pub type DeltaSink = Arc<dyn Fn(StreamDelta) + Send + Sync>;
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn route(&self) -> Option<ModelRoute> {
        None
    }
    fn request_snapshot(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
    ) -> AppResult<Value> {
        Ok(serde_json::json!({"messages":messages,"tools":tools}))
    }
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
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
    fn route(&self) -> Option<ModelRoute> {
        Some(ModelRoute::from(&self.config))
    }
    fn request_snapshot(
        &self,
        messages: &[ModelMessage],
        tools: &[ToolDefinition],
    ) -> AppResult<Value> {
        let (url, body) = encode_request(&self.config, messages, tools)?;
        Ok(serde_json::json!({"url":url,"body":body}))
    }
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        tools: Vec<ToolDefinition>,
        cancel: CancellationToken,
        sink: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = self.config.proxy.as_ref().filter(|p| !p.is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|_| AppError::new("MODEL_PROXY", "代理地址无效"))?,
            );
        }
        let client = builder.build().map_err(transport_error)?;
        let (url, body) = encode_request(&self.config, &messages, &tools)?;
        let mut request = client
            .post(url)
            .json(&body)
            .timeout(Duration::from_secs(180));
        match self.config.api_mode {
            ApiMode::Anthropic => {
                request = request.header("anthropic-version", "2023-06-01");
                if !self.secret.is_empty() {
                    request = request.header("x-api-key", &self.secret);
                }
            }
            _ => {
                if !self.secret.is_empty() {
                    request = request.bearer_auth(&self.secret);
                }
            }
        }
        let response = tokio::select! {_=cancel.cancelled()=>return Err(cancelled()),r=request.send()=>r.map_err(transport_error)?};
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(response, cancel.clone()).await?);
        }
        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::for_route(ModelRoute::from(&self.config));
        while let Some(chunk) =
            tokio::select! {_=cancel.cancelled()=>return Err(cancelled()),next=stream.next()=>next}
        {
            for delta in decoder.push(&chunk.map_err(transport_error)?)? {
                sink(delta);
            }
            if decoder.is_finished() {
                break;
            }
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        decoder.finish()
    }
}
pub fn cancelled() -> AppError {
    AppError::new("CANCELLED", "操作已取消")
}
pub(crate) fn transport_error(e: reqwest::Error) -> AppError {
    AppError::new(
        if e.is_timeout() {
            "MODEL_TIMEOUT"
        } else {
            "MODEL_TRANSPORT"
        },
        if e.is_timeout() {
            "模型请求超时"
        } else {
            "无法连接或读取模型服务；请检查网络和代理"
        },
    )
}

// Only whitelisted error identifiers affect classification; provider text is never exposed.
async fn http_error(response: reqwest::Response, cancel: CancellationToken) -> AppResult<AppError> {
    let status = response.status().as_u16();
    let mut code = match status {
        401 | 403 => "MODEL_AUTH",
        404 | 405 => "MODEL_ENDPOINT",
        400 | 422 => "MODEL_REQUEST",
        429 => "MODEL_RATE_LIMIT",
        _ => "MODEL_HTTP",
    };
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) =
        tokio::select! {_=cancel.cancelled()=>return Err(cancelled()),next=stream.next()=>next}
    {
        let Ok(chunk) = chunk else { break };
        if bytes.len() + chunk.len() > 16 * 1024 {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        let error = &value["error"];
        let kind = error["code"]
            .as_str()
            .or_else(|| error["type"].as_str())
            .unwrap_or("");
        if matches!(kind, "model_not_found" | "invalid_model" | "unknown_model")
            || error["param"] == "model"
        {
            code = "MODEL_ID";
        } else if matches!(
            kind,
            "invalid_api_key" | "authentication_error" | "permission_error"
        ) {
            code = "MODEL_AUTH";
        } else if matches!(
            kind,
            "unsupported_protocol" | "invalid_api_version" | "unsupported_api"
        ) {
            code = "MODEL_PROTOCOL";
        }
    }
    let message = match code {
        "MODEL_AUTH" => "供应商认证失败，请检查该供应商的 API Key 和权限",
        "MODEL_ID" => "模型不可用，请检查模型 ID 及账号模型权限",
        "MODEL_ENDPOINT" => "协议端点不可用，请检查供应商及协议选择",
        "MODEL_PROTOCOL" => "供应商不支持当前协议或协议版本",
        "MODEL_RATE_LIMIT" => "供应商限流或额度不足",
        _ => "模型请求失败，请检查协议参数和服务状态",
    };
    Ok(AppError::new(code, format!("{message}（HTTP {status}）")))
}
