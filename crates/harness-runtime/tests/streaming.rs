use harness_contracts::ApiMode;
use harness_runtime::model::{ModelRoute, SseDecoder};

#[test]
fn fragmented_utf8_and_tool_arguments_are_reassembled() {
    let text = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\",\"reasoning_content\":\"分析\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let mut decoder = SseDecoder::default();
    for byte in text.as_bytes() {
        decoder.push(&[*byte]).unwrap();
    }
    let result = decoder.finish().unwrap();
    assert_eq!(result.content, "你好");
    assert_eq!(result.reasoning, "分析");
    assert_eq!(result.calls[0].name, "read");
    assert_eq!(result.calls[0].arguments["path"], "a.txt");
}

#[test]
fn broken_stream_never_becomes_a_successful_message() {
    let mut decoder = SseDecoder::default();
    decoder
        .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n")
        .unwrap();
    assert_eq!(decoder.finish().unwrap_err().code, "INCOMPLETE_STREAM");
}

#[test]
fn multiline_sse_data_is_one_event() {
    let mut decoder = SseDecoder::default();
    decoder.push(b": heartbeat\r\ndata: {\"choices\":\r\ndata: [{\"delta\":{\"content\":\"joined\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n").unwrap();
    assert_eq!(decoder.finish().unwrap().content, "joined");
}

#[test]
fn token_limit_is_not_success_even_with_done_marker() {
    let mut decoder = SseDecoder::default();
    let result = decoder.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n");
    let error = match result {
        Err(error) => error,
        Ok(_) => decoder.finish().unwrap_err(),
    };
    assert_eq!(error.code, "MODEL_TRUNCATED");
}

#[test]
fn anthropic_cut_mid_frame_reports_stream_interruption() {
    // 复现线上事故形态：thinking/text 块完整收尾后，tool_use 参数流到一半连接被关闭，
    // EOF 残留半行 data 帧应归为流中断而非协议错误。
    let route = ModelRoute {
        provider_id: "bigmodel".into(),
        api_mode: ApiMode::Anthropic,
        model: "glm-5.3-flash".into(),
        base_url: "https://example.invalid/api/anthropic".into(),
    };
    let mut decoder = SseDecoder::for_route(route);
    let prefix = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"正文\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool_1\",\"name\":\"plan_write\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"plan\\\":\"}}\n\n",
    );
    decoder.push(prefix.as_bytes()).unwrap();
    decoder
        .push(b"data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_")
        .unwrap();
    let error = decoder.finish().unwrap_err();
    assert_eq!(error.code, "INCOMPLETE_STREAM");
    assert!(error.message.contains("中断"));
    assert_ne!(error.message, "模型流不是完整 JSON");
}

#[test]
fn malformed_frame_mid_stream_stays_protocol_error() {
    let mut decoder = SseDecoder::default();
    let error = decoder.push(b"data: not-json\n\n").unwrap_err();
    assert_eq!(error.code, "MODEL_PROTOCOL");
    assert_eq!(error.message, "模型流不是完整 JSON");
}
