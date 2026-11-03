use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
#[test]
fn real_stdio_handles_parse_errors_then_serves_requests() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["--data-dir", dir.path().to_str().unwrap(), "rpc"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    writeln!(input, "bad json").unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let error: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(error["error"]["code"], -32700);
    writeln!(
        input,
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"workspace.list\"}}"
    )
    .unwrap();
    input.flush().unwrap();
    line.clear();
    out.read_line(&mut line).unwrap();
    let result: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["id"], 7);
    assert_eq!(result["result"], serde_json::json!([]));
    drop(input);
    assert!(child.wait().unwrap().success());
}
