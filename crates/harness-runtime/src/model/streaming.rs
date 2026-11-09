//! Shared SSE framing, with protocol-specific completion and opaque replay.
use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

const FRAME_LIMIT: usize = 4 * 1024 * 1024;
const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
fn protocol_error(message: &str) -> AppError {
    AppError::new("MODEL_PROTOCOL", message)
}
fn truncated() -> AppError {
    AppError::new(
        "MODEL_TRUNCATED",
        "模型输出被截断；请提高输出长度或缩短上下文",
    )
}
/// 流结束时仍有残留半行帧：连接被提前关闭，属于传输中断而非协议错误。
fn stream_interrupted() -> AppError {
    AppError::new(
        "INCOMPLETE_STREAM",
        "模型流中断（连接提前关闭，末尾数据不完整）",
    )
}
fn parse_json(text: &str) -> AppResult<Value> {
    serde_json::from_str(text).map_err(|_| protocol_error("模型流不是完整 JSON"))
}

#[derive(Default)]
struct Frames {
    buffer: Vec<u8>,
    data: Vec<String>,
    event: String,
    size: usize,
}
impl Frames {
    fn line(&mut self, bytes: &[u8], output: &mut Vec<(String, String)>) -> AppResult<()> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| protocol_error("模型流包含无效 UTF-8"))?
            .trim_end_matches('\r');
        if text.is_empty() {
            if !self.data.is_empty() {
                output.push((std::mem::take(&mut self.event), self.data.join("\n")));
                self.data.clear();
            }
            self.event.clear();
            self.size = 0;
        } else if let Some(data) = text.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            self.size += data.len();
            if self.size > FRAME_LIMIT {
                return Err(AppError::new("MODEL_LIMIT", "模型单帧超过4MiB"));
            }
            self.data.push(data.into());
        } else if let Some(event) = text.strip_prefix("event:") {
            self.event = event.trim().into();
        }
        Ok(())
    }
    fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<(String, String)>> {
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            if pos > FRAME_LIMIT {
                return Err(AppError::new("MODEL_LIMIT", "模型单行超过4MiB"));
            }
            let mut line: Vec<u8> = self.buffer.drain(..=pos).collect();
            line.pop();
            self.line(&line, &mut output)?;
        }
        if self.buffer.len() > FRAME_LIMIT {
            return Err(AppError::new("MODEL_LIMIT", "模型单行超过4MiB"));
        }
        Ok(output)
    }
    fn finish(&mut self) -> AppResult<Vec<(String, String)>> {
        let mut output = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.line(&line, &mut output)?;
        }
        self.line(b"", &mut output)?;
        Ok(output)
    }
    /// EOF 时是否残留未成行的字节（连接在帧中间被关闭）。
    fn has_residual(&self) -> bool {
        !self.buffer.is_empty()
    }
}
#[derive(Default)]
pub struct SseDecoder {
    frames: Frames,
    route: Option<ModelRoute>,
    answer: ModelAnswer,
    calls: BTreeMap<u64, (String, String, String)>,
    blocks: BTreeMap<u64, Value>,
    inputs: BTreeMap<u64, String>,
    open_blocks: BTreeSet<u64>,
    stop_reason: Option<String>,
    finished: bool,
    received: usize,
}
impl SseDecoder {
    pub fn for_route(route: ModelRoute) -> Self {
        Self {
            route: Some(route),
            ..Default::default()
        }
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    pub(crate) fn usage(&self) -> Value {
        self.answer.usage.clone()
    }
    fn mode(&self) -> ApiMode {
        self.route.as_ref().map(|r| r.api_mode).unwrap_or_default()
    }
    pub fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<StreamDelta>> {
        self.received = self.received.saturating_add(bytes.len());
        if self.received > RESPONSE_LIMIT {
            return Err(AppError::new("MODEL_LIMIT", "模型响应超过32MiB"));
        }
        let frames = self.frames.push(bytes)?;
        self.events(frames)
    }
    fn events(&mut self, frames: Vec<(String, String)>) -> AppResult<Vec<StreamDelta>> {
        let mut output = Vec::new();
        for (event, data) in frames {
            if self.finished {
                return Err(protocol_error("模型结束后仍返回数据"));
            }
            if data == "[DONE]" && self.mode() == ApiMode::ChatCompletions {
                self.finished = true;
                continue;
            }
            if data.trim().is_empty() {
                continue;
            }
            let mut v = parse_json(&data)?;
            if v.get("type").is_none() && !event.is_empty() {
                v["type"] = json!(event);
            }
            if v.get("error").is_some() || v["type"] == "error" {
                return Err(AppError::new("MODEL_STREAM", "模型服务返回流式错误"));
            }
            match self.mode() {
                ApiMode::ChatCompletions => self.chat(&v, &mut output)?,
                ApiMode::Responses => self.responses(&v, &mut output)?,
                ApiMode::Anthropic => self.anthropic(&v, &mut output)?,
            }
        }
        Ok(output)
    }
    fn delta(&mut self, text: &str, reasoning: bool, output: &mut Vec<StreamDelta>) {
        if reasoning {
            self.answer.reasoning.push_str(text);
        } else {
            self.answer.content.push_str(text);
        }
        output.push(StreamDelta {
            text: text.into(),
            reasoning,
        });
    }
    fn chat(&mut self, v: &Value, output: &mut Vec<StreamDelta>) -> AppResult<()> {
        if !v["usage"].is_null() {
            self.answer.usage = v["usage"].clone();
        }
        for choice in v["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["index"].as_u64().unwrap_or(0) == 0)
        {
            if let Some(reason) = choice["finish_reason"].as_str() {
                match reason {
                    "length" => return Err(truncated()),
                    "content_filter" => {
                        return Err(AppError::new("MODEL_REFUSAL", "模型拒绝了请求"));
                    }
                    "stop" | "tool_calls" => {}
                    _ => return Err(protocol_error("不支持的模型结束原因")),
                }
            }
            let delta = &choice["delta"];
            for (key, reasoning) in [("content", false), ("reasoning_content", true)] {
                if let Some(text) = delta[key].as_str() {
                    self.delta(text, reasoning, output);
                }
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                let slot = self
                    .calls
                    .entry(call["index"].as_u64().unwrap_or(0))
                    .or_default();
                if let Some(id) = call["id"].as_str() {
                    slot.0.push_str(id);
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    slot.1.push_str(name);
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    slot.2.push_str(args);
                }
            }
        }
        Ok(())
    }
    fn responses(&mut self, v: &Value, output: &mut Vec<StreamDelta>) -> AppResult<()> {
        match v["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(text) = v["delta"].as_str() {
                    self.delta(text, false, output);
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                if let Some(text) = v["delta"].as_str() {
                    self.delta(text, true, output);
                }
            }
            "response.output_item.done" => {
                let index = v["output_index"]
                    .as_u64()
                    .ok_or_else(|| protocol_error("输出项缺少索引"))?;
                self.blocks.insert(index, v["item"].clone());
            }
            "response.incomplete" => return Err(truncated()),
            "response.failed" => return Err(AppError::new("MODEL_STREAM", "Responses 请求失败")),
            "response.completed" => {
                let response = &v["response"];
                if response["status"] != "completed" {
                    return Err(protocol_error("Responses 未完成"));
                }
                if let Some(items) = response["output"].as_array() {
                    self.blocks = items
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(i, v)| (i as u64, v))
                        .collect();
                } else {
                    return Err(protocol_error("Responses 完成事件缺少输出项"));
                }
                self.answer.usage = response["usage"].clone();
                self.finished = true;
            }
            _ => {}
        }
        Ok(())
    }
    fn anthropic(&mut self, v: &Value, output: &mut Vec<StreamDelta>) -> AppResult<()> {
        let index = v["index"].as_u64().unwrap_or(0);
        match v["type"].as_str().unwrap_or("") {
            "message_start" => self.answer.usage = v["message"]["usage"].clone(),
            "content_block_start" => {
                self.open_blocks.insert(index);
                if self
                    .blocks
                    .insert(index, v["content_block"].clone())
                    .is_some()
                {
                    return Err(protocol_error("重复内容块索引"));
                }
                let block = &v["content_block"];
                for (key, reasoning) in [("text", false), ("thinking", true)] {
                    if let Some(text) = block[key].as_str().filter(|s| !s.is_empty()) {
                        self.delta(text, reasoning, output);
                    }
                }
            }
            "content_block_delta" => {
                if !self.open_blocks.contains(&index) {
                    return Err(protocol_error("内容块尚未开始或已关闭"));
                }
                let delta = &v["delta"];
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or_else(|| protocol_error("内容增量没有起始块"))?;
                let (field, text, reasoning) = match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => ("text", delta["text"].as_str(), false),
                    "thinking_delta" => ("thinking", delta["thinking"].as_str(), true),
                    "signature_delta" => ("signature", delta["signature"].as_str(), true),
                    "input_json_delta" => {
                        self.inputs
                            .entry(index)
                            .or_default()
                            .push_str(delta["partial_json"].as_str().unwrap_or(""));
                        return Ok(());
                    }
                    _ => return Ok(()),
                };
                if let Some(text) = text {
                    let mut value = block[field].as_str().unwrap_or("").to_owned();
                    value.push_str(text);
                    block[field] = json!(value);
                    if field != "signature" {
                        self.delta(text, reasoning, output);
                    }
                }
            }
            "content_block_stop" => {
                if !self.open_blocks.remove(&index) {
                    return Err(protocol_error("内容块尚未开始或已关闭"));
                }
                if let Some(input) = self.inputs.remove(&index) {
                    let block = self
                        .blocks
                        .get_mut(&index)
                        .ok_or_else(|| protocol_error("工具块缺失"))?;
                    block["input"] = parse_json(&input)?;
                }
            }
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    match reason {
                        "max_tokens" | "model_context_window_exceeded" => return Err(truncated()),
                        "end_turn" | "tool_use" | "stop_sequence" => {}
                        _ => return Err(protocol_error("不支持的 Anthropic 结束原因")),
                    }
                    self.stop_reason = Some(reason.into());
                }
                if let Some(usage) = v["usage"].as_object() {
                    if !self.answer.usage.is_object() {
                        self.answer.usage = json!({});
                    }
                    for (key, value) in usage {
                        self.answer.usage[key] = value.clone();
                    }
                }
            }
            "message_stop" => {
                if self.stop_reason.is_none()
                    || !self.inputs.is_empty()
                    || !self.open_blocks.is_empty()
                {
                    return Err(protocol_error(
                        "Anthropic 完成事件缺少结束原因或工具块未完成",
                    ));
                }
                self.finished = true;
            }
            _ => {}
        }
        Ok(())
    }
    pub fn finish(self) -> AppResult<ModelAnswer> {
        self.finish_with_usage().map_err(|(error, _)| error)
    }
    pub(crate) fn finish_with_usage(mut self) -> Result<ModelAnswer, (AppError, Value)> {
        // EOF 残留半行帧触发的解析失败是连接被提前关闭，归为流中断；
        // 完整帧仍解析失败的才保留协议错误语义。
        let residual = self.frames.has_residual();
        match self.finish_inner() {
            Ok(()) => Ok(self.answer),
            Err(error) => {
                let error = if residual && error.code == "MODEL_PROTOCOL" {
                    stream_interrupted()
                } else {
                    error
                };
                Err((error, self.answer.usage.clone()))
            }
        }
    }
    fn finish_inner(&mut self) -> AppResult<()> {
        let frames = self.frames.finish()?;
        // Final frames may contain a terminal marker without a trailing newline.
        self.events(frames)?;
        if !self.finished {
            return Err(AppError::new(
                "INCOMPLETE_STREAM",
                "模型流中断，未收到结束标记",
            ));
        }
        let mut replay = Vec::new();
        match self.mode() {
            ApiMode::ChatCompletions => {
                for (_, (id, name, args)) in std::mem::take(&mut self.calls) {
                    if id.is_empty() || name.is_empty() {
                        return Err(protocol_error("工具调用缺少标识或名称"));
                    }
                    self.answer.calls.push(ToolCall {
                        id,
                        name,
                        arguments: parse_json(&args)?,
                    });
                }
            }
            ApiMode::Responses => {
                let mut content = String::new();
                let mut reasoning = String::new();
                for item in std::mem::take(&mut self.blocks).into_values() {
                    match item["type"].as_str().unwrap_or("") {
                        "function_call" => self.answer.calls.push(ToolCall {
                            id: required(&item, "call_id")?,
                            name: required(&item, "name")?,
                            arguments: parse_json(
                                item["arguments"]
                                    .as_str()
                                    .ok_or_else(|| protocol_error("函数参数缺失"))?,
                            )?,
                        }),
                        "message" => {
                            for part in item["content"].as_array().into_iter().flatten() {
                                if part["type"] == "refusal" {
                                    return Err(AppError::new("MODEL_REFUSAL", "模型拒绝了请求"));
                                }
                                if let Some(text) = part["text"].as_str() {
                                    content.push_str(text);
                                }
                            }
                        }
                        "reasoning" => {
                            for part in item["content"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .chain(item["summary"].as_array().into_iter().flatten())
                            {
                                if let Some(text) = part["text"].as_str() {
                                    reasoning.push_str(text);
                                }
                            }
                        }
                        _ => return Err(protocol_error("Responses 返回了未请求的输出项类型")),
                    }
                    replay.push(item);
                }
                if !content.is_empty() {
                    self.answer.content = content;
                }
                if !reasoning.is_empty() {
                    self.answer.reasoning = reasoning;
                }
            }
            ApiMode::Anthropic => {
                for block in std::mem::take(&mut self.blocks).into_values() {
                    match block["type"].as_str().unwrap_or("") {
                        "tool_use" => self.answer.calls.push(ToolCall {
                            id: required(&block, "id")?,
                            name: required(&block, "name")?,
                            arguments: block["input"].clone(),
                        }),
                        "text" | "thinking" | "redacted_thinking" => {}
                        _ => return Err(protocol_error("Anthropic 返回了未请求的内容块类型")),
                    }
                    replay.push(block);
                }
            }
        }
        if let Some(route) = self.route.take() {
            self.answer.replay = Some(ProtocolReplay {
                route,
                items: replay,
            });
        }
        Ok(())
    }
}
fn required(v: &Value, key: &str) -> AppResult<String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| protocol_error("工具调用缺少标识或名称"))
}
