use harness_contracts::*;
use harness_runtime::model::*;
use serde_json::{Value, json};

fn config(mode: ApiMode) -> ModelConfig {
    serde_json::from_value(json!({"id":"test","name":"test","baseUrl":"https://example.test/api/v1","model":"model","credentialRef":"","proxy":null,"providerId":"custom","apiMode":mode,"maxOutputTokens":512})).unwrap()
}
fn route(mode: ApiMode) -> ModelRoute {
    ModelRoute::from(&config(mode))
}
fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}

#[test]
fn encoders_use_distinct_wire_shapes_and_preserve_prefixes() {
    let messages = vec![ModelMessage::system("rules"), ModelMessage::user("hello")];
    let tool = ToolDefinition {
        name: "read".into(),
        description: "read file".into(),
        parameters: json!({"type":"object","properties":{}}),
    };
    for (mode, suffix) in [
        (ApiMode::ChatCompletions, "/chat/completions"),
        (ApiMode::Responses, "/responses"),
        (ApiMode::Anthropic, "/messages"),
    ] {
        let (url, body) =
            encode_request(&config(mode), &messages, std::slice::from_ref(&tool)).unwrap();
        assert_eq!(url, format!("https://example.test/api/v1{suffix}"));
        match mode {
            ApiMode::ChatCompletions => {
                assert_eq!(body["messages"][0]["role"], "system");
                assert_eq!(body["tools"][0]["function"]["name"], "read");
            }
            ApiMode::Responses => {
                assert_eq!(body["input"][0]["role"], "system");
                assert_eq!(body["tools"][0]["name"], "read");
                assert_eq!(body["store"], false);
                assert!(body.get("previous_response_id").is_none());
            }
            ApiMode::Anthropic => {
                assert_eq!(body["system"], "rules");
                assert_eq!(body["messages"][0]["role"], "user");
                assert_eq!(body["tools"][0]["name"], "read");
                assert_eq!(body["max_tokens"], 512);
            }
        }
    }
}

#[test]
fn responses_complete_replays_encrypted_reasoning_and_call_id_only_on_same_route() {
    let output = json!([{"type":"reasoning","id":"r","encrypted_content":"opaque"},{"type":"function_call","id":"item-id","call_id":"call-id","name":"read","arguments":"{\"path\":\"a\"}"}]);
    let mut decoder = SseDecoder::for_route(route(ApiMode::Responses));
    let stream = frame(
        json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":5}}}),
    );
    for b in stream.as_bytes() {
        decoder.push(&[*b]).unwrap();
    }
    let answer = decoder.finish().unwrap();
    assert_eq!(answer.calls[0].id, "call-id");
    let message = ModelMessage {
        role: MessageRole::Assistant,
        tool_calls: answer.calls,
        replay: answer.replay,
        ..Default::default()
    };
    let mut history = vec![message, ModelMessage::tool("call-id", "contents", false)];
    let (_, body) = encode_request(&config(ApiMode::Responses), &history, &[]).unwrap();
    assert_eq!(body["input"][0]["encrypted_content"], "opaque");
    assert_eq!(body["input"][2]["call_id"], "call-id");
    let (_, other) = encode_request(&config(ApiMode::Anthropic), &history, &[]).unwrap();
    assert!(!other.to_string().contains("opaque"));
    history[0].replay.as_mut().unwrap().route.provider_id = "different".into();
    let (_, other) = encode_request(&config(ApiMode::Responses), &history, &[]).unwrap();
    assert!(!other.to_string().contains("opaque"));
    assert_eq!(other["input"][0]["call_id"], "call-id");
}

#[test]
fn anthropic_signed_thinking_parallel_tools_and_results_round_trip() {
    let mut decoder = SseDecoder::for_route(route(ApiMode::Anthropic));
    let events = vec![
        json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"a","name":"read","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"b","name":"read","input":{"path":"b"}}}),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}),
        json!({"type":"message_stop"}),
    ];
    for event in events {
        for byte in frame(event).as_bytes() {
            decoder.push(&[*byte]).unwrap();
        }
    }
    let answer = decoder.finish().unwrap();
    assert_eq!(answer.calls.len(), 2);
    assert_eq!(answer.usage["input_tokens"], 9);
    let history = vec![
        ModelMessage {
            role: MessageRole::Assistant,
            reasoning: answer.reasoning,
            tool_calls: answer.calls,
            replay: answer.replay,
            ..Default::default()
        },
        ModelMessage::tool("a", "A", false),
        ModelMessage::tool("b", "B", true),
    ];
    let (_, body) = encode_request(&config(ApiMode::Anthropic), &history, &[]).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["signature"], "signed");
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    assert_eq!(body["messages"][1]["content"][1]["is_error"], true);
}

#[test]
fn protocol_failure_events_never_settle_as_success() {
    for (mode, event) in [
        (
            ApiMode::Responses,
            json!({"type":"response.incomplete","response":{"status":"incomplete"}}),
        ),
        (
            ApiMode::Responses,
            json!({"type":"response.failed","response":{"status":"failed"}}),
        ),
        (
            ApiMode::Anthropic,
            json!({"type":"error","error":{"type":"overloaded_error"}}),
        ),
        (
            ApiMode::Anthropic,
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
        ),
    ] {
        let mut decoder = SseDecoder::for_route(route(mode));
        let result = decoder.push(frame(event).as_bytes());
        assert!(result.is_err() || decoder.finish().is_err());
    }
}

#[test]
fn six_builtin_routes_use_the_exact_fixed_prefixes() {
    let expected = [
        (
            "deepseek",
            ApiMode::ChatCompletions,
            "https://api.deepseek.com/chat/completions",
        ),
        (
            "deepseek",
            ApiMode::Responses,
            "https://api.deepseek.com/responses",
        ),
        (
            "deepseek",
            ApiMode::Anthropic,
            "https://api.deepseek.com/anthropic/v1/messages",
        ),
        (
            "bigmodel",
            ApiMode::ChatCompletions,
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions",
        ),
        (
            "bigmodel",
            ApiMode::Responses,
            "https://open.bigmodel.cn/api/v1/responses",
        ),
        (
            "bigmodel",
            ApiMode::Anthropic,
            "https://open.bigmodel.cn/api/anthropic/v1/messages",
        ),
    ];
    let providers = harness_runtime::providers::builtin_providers();
    assert_eq!(providers.len(), 2);
    for (id, mode, url) in expected {
        let p = providers.iter().find(|p| p.id == id).unwrap();
        let mut c = config(mode);
        c.provider_id = id.into();
        c.compatibility = p.compatibility;
        c.base_url = p
            .endpoints
            .iter()
            .find(|e| e.api_mode == mode)
            .unwrap()
            .base_url
            .clone();
        let (actual, body) = encode_request(&c, &[ModelMessage::user("hello")], &[]).unwrap();
        assert_eq!(actual, url);
        if id == "deepseek" && mode == ApiMode::Responses {
            assert!(body.get("include").is_none());
            assert!(body.get("store").is_none());
            assert!(body.get("stream_options").is_none());
        }
    }
}

#[test]
fn anthropic_requires_closed_content_blocks_before_success() {
    for closing in [false, true] {
        let mut decoder = SseDecoder::for_route(route(ApiMode::Anthropic));
        decoder.push(frame(json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"list_files","input":{}}})).as_bytes()).unwrap();
        if closing {
            decoder
                .push(frame(json!({"type":"content_block_stop","index":0})).as_bytes())
                .unwrap();
            assert!(decoder.push(frame(json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}})).as_bytes()).is_err());
        } else {
            decoder
                .push(
                    frame(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}))
                        .as_bytes(),
                )
                .unwrap();
            assert!(
                decoder
                    .push(frame(json!({"type":"message_stop"})).as_bytes())
                    .is_err()
            );
        }
    }
}

#[test]
fn responses_cross_protocol_assistant_text_uses_easy_input_message() {
    let message = ModelMessage {
        role: MessageRole::Assistant,
        content: "previous answer".into(),
        ..Default::default()
    };
    let (_, body) = encode_request(&config(ApiMode::Responses), &[message], &[]).unwrap();
    assert_eq!(
        body["input"][0],
        json!({"role":"assistant","content":"previous answer"})
    );
}
