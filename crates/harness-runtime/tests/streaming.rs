use harness_runtime::model::SseDecoder;

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
