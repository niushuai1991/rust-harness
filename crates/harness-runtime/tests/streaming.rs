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
