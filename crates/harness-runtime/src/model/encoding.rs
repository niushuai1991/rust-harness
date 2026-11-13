use super::*;
use serde_json::json;

pub fn encode_request(
    config: &ModelConfig,
    messages: &[ModelMessage],
    tools: &[ToolDefinition],
) -> AppResult<(String, Value)> {
    let route = ModelRoute::from(config);

    let base = config.base_url.trim_end_matches('/');
    let suffix = match config.api_mode {
        ApiMode::ChatCompletions => "/chat/completions",
        ApiMode::Responses => "/responses",
        ApiMode::Anthropic => {
            if base.ends_with("/v1") {
                "/messages"
            } else {
                "/v1/messages"
            }
        }
    };
    let url = format!("{base}{suffix}");
    let parsed =
        reqwest::Url::parse(&url).map_err(|_| AppError::new("MODEL_URL", "服务地址无效"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::new(
            "MODEL_URL",
            "请输入不含凭据、查询参数和片段的 HTTP(S) Base URL",
        ));
    }
    let mut body = json!({"model":config.model,"stream":true});
    match config.api_mode {
        ApiMode::ChatCompletions => {
            body["messages"]=json!(messages.iter().map(|m|{
                let mut v=json!({"role":m.role,"content":m.content});
                if m.role==MessageRole::Assistant {
                    if replay_for(m,&route).is_some()&&!m.reasoning.is_empty(){v["reasoning_content"]=json!(m.reasoning);}
                    if !m.tool_calls.is_empty(){v["tool_calls"]=json!(m.tool_calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}})).collect::<Vec<_>>());}
                }
                if m.role==MessageRole::Tool {v["tool_call_id"]=json!(m.tool_call_id);}
                v
            }).collect::<Vec<_>>());
            body["stream_options"] = json!({"include_usage":true});
            if let Some(limit) = config.max_output_tokens {
                body["max_tokens"] = json!(limit);
            }
            if !tools.is_empty() {
                body["tools"]=json!(tools.iter().map(|t|json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}})).collect::<Vec<_>>());
            }
        }
        ApiMode::Responses => {
            let mut input = Vec::new();
            for m in messages {
                if m.role == MessageRole::Assistant {
                    if let Some(replay) = replay_for(m, &route).filter(|r| !r.items.is_empty()) {
                        input.extend(replay.items.clone());
                        continue;
                    }
                    if !m.content.is_empty() {
                        input.push(json!({"role":"assistant","content":m.content}));
                    }
                    for c in &m.tool_calls {
                        input.push(json!({"type":"function_call","call_id":c.id,"name":c.name,"arguments":c.arguments.to_string()}));
                    }
                } else if m.role == MessageRole::Tool {
                    input.push(json!({"type":"function_call_output","call_id":m.tool_call_id,"output":m.content}));
                } else {
                    // ChatGPT 后端要求基础指令以 developer 消息进入 input，而非 system。
                    let role = match m.role {
                        MessageRole::System if config.auth == ProviderAuth::ChatGptOAuth => {
                            "developer"
                        }
                        MessageRole::System => "system",
                        _ => "user",
                    };
                    input.push(json!({"role":role,"content":m.content}));
                }
            }
            body["input"] = json!(input);
            if config.compatibility != ProviderCompatibility::DeepSeek {
                body["store"] = json!(false);
            }
            if config.compatibility == ProviderCompatibility::Standard {
                body["include"] = json!(["reasoning.encrypted_content"]);
            }
            // ChatGPT 后端不消费 max_output_tokens（codex 亦不发送）。
            if let Some(limit) = config
                .max_output_tokens
                .filter(|_| config.auth != ProviderAuth::ChatGptOAuth)
            {
                body["max_output_tokens"] = json!(limit);
            }
            if !tools.is_empty() {
                body["tools"]=json!(tools.iter().map(|t|json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters,"strict":false})).collect::<Vec<_>>());
            }
        }
        ApiMode::Anthropic => {
            body["system"] = json!(
                messages
                    .iter()
                    .filter(|m| m.role == MessageRole::System)
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            );
            let mut output: Vec<Value> = Vec::new();
            for m in messages.iter().filter(|m| m.role != MessageRole::System) {
                let role = if m.role == MessageRole::Assistant {
                    "assistant"
                } else {
                    "user"
                };
                let mut blocks = Vec::new();
                if m.role == MessageRole::Tool {
                    blocks.push(json!({"type":"tool_result","tool_use_id":m.tool_call_id,"content":m.content,"is_error":m.is_error}));
                } else if let Some(replay) = replay_for(m, &route).filter(|r| !r.items.is_empty()) {
                    blocks.extend(replay.items.clone());
                } else {
                    if !m.content.is_empty() {
                        blocks.push(json!({"type":"text","text":m.content}));
                    }
                    for c in &m.tool_calls {
                        blocks.push(
                            json!({"type":"tool_use","id":c.id,"name":c.name,"input":c.arguments}),
                        );
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                if let Some(last) = output.last_mut().filter(|v| v["role"] == role) {
                    last["content"].as_array_mut().unwrap().extend(blocks);
                } else {
                    output.push(json!({"role":role,"content":blocks}));
                }
            }
            body["messages"] = json!(output);
            body["max_tokens"] = json!(config.max_output_tokens.unwrap_or(4096));
            if !tools.is_empty() {
                body["tools"]=json!(tools.iter().map(|t|json!({"name":t.name,"description":t.description,"input_schema":t.parameters})).collect::<Vec<_>>());
            }
        }
    }
    // 推理档位按协议方言注入；None（跟随默认）不发送任何字段。
    // 映射与内置 modelApiRules 保持一致。
    if let Some(effort) = config.effort.as_deref() {
        match config.api_mode {
            ApiMode::Anthropic => {
                if effort == "disabled" || effort == "none" {
                    body["thinking"] = json!({"type":"disabled"});
                } else {
                    body["thinking"] = json!({"type":"adaptive"});
                    body["output_config"] = json!({"effort":effort});
                }
            }
            ApiMode::ChatCompletions => {
                let off = effort == "disabled" || effort == "none";
                body["thinking"] = json!({"type": if off { "disabled" } else { "enabled" }});
                body["enable_thinking"] = json!(!off);
                let level = if off {
                    "none"
                } else if effort == "enabled" {
                    "high"
                } else {
                    effort
                };
                body["reasoning_effort"] = json!(level);
                body["reasoning"] = json!({"effort":level});
            }
            ApiMode::Responses => {
                if effort != "disabled" && effort != "none" {
                    body["reasoning"] = json!({"effort":effort});
                }
            }
        }
    }
    Ok((url, body))
}

fn replay_for<'a>(message: &'a ModelMessage, route: &ModelRoute) -> Option<&'a ProtocolReplay> {
    message.replay.as_ref().filter(|r| r.route == *route)
}
