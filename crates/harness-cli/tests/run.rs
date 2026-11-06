//! `harness run` 集成测试：本地 SSE mock 驱动，验证 JSONL 契约、退出码与 stdin 矩阵。
use std::process::Stdio;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};

/// ChatCompletions mock：每个请求都回复流式 "finished"（无工具调用）。
async fn mock_finished() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buffer = [0u8; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break bytes.len();
                }
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_lowercase();
            if let Some(length) = headers.lines().find_map(|line| {
                let (k, v) = line.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            }) {
                while bytes.len() < header_end + length {
                    let mut buffer = [0u8; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
            }
            let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"finished\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        }
    });
    base
}

fn harness(data_dir: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harness"));
    command.arg("--data-dir").arg(data_dir);
    command
}

fn host_dir(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("host")
}

async fn configure_model(data_dir: &std::path::Path, base: &str) {
    for (method, params) in [
        (
            "provider.save",
            format!(
                r#"{{"config":{{"id":"fixture","name":"fixture","builtin":false,"credentialRef":"test/none","defaultModel":"mock","endpoints":[{{"apiMode":"chatCompletions","baseUrl":"{base}"}}]}}}}"#
            ),
        ),
        (
            "model.save",
            format!(
                r#"{{"config":{{"id":"mock","name":"mock","providerId":"fixture","apiMode":"chatCompletions","model":"model","credentialRef":"test/none","baseUrl":"{base}","maxOutputTokens":1024}}}}"#
            ),
        ),
    ] {
        let output = harness(data_dir)
            .args(["call", method, &params])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{method} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn source_dir(dir: &std::path::Path) -> std::path::PathBuf {
    let source = dir.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    source
}

#[tokio::test]
async fn run_json_streams_deltas_and_reports_result_line() {
    let base = mock_finished().await;
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    configure_model(&host_dir(&dir), &base).await;
    let output = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "auto",
            "--json",
            "-m",
            "mock",
            "总结项目",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty());
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("每行必须可独立解析：{error}：{line}"))
        })
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line["type"] == "delta" && line["text"] == "finished")
    );
    let result = lines.last().unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["exitCode"], 0);
    assert_eq!(result["finalMessage"], "finished");
    assert!(result["sessionId"].is_string());
    assert!(result["workspaceId"].is_string());
    assert!(result["messageId"].is_string());
    assert_eq!(result["changedFiles"], 0);
}

#[tokio::test]
async fn run_reads_prompt_from_piped_stdin_when_missing() {
    let base = mock_finished().await;
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    configure_model(&host_dir(&dir), &base).await;
    let mut child = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "auto",
            "--json",
            "-m",
            "mock",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all("总结项目\n".as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(result["status"], "completed");
    assert_eq!(result["finalMessage"], "finished");
}

#[tokio::test]
async fn run_without_explicit_approval_rejects_piped_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    let mut child = harness(&host_dir(&dir))
        .args(["run", "-C", source.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all("任务\n".as_bytes()).await;
    let _ = stdin.flush().await;
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--approval"), "stderr: {stderr}");
}

#[tokio::test]
async fn run_ask_policy_requires_tty() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    let output = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "ask",
            "任务",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TTY"), "stderr: {stderr}");
}

#[tokio::test]
async fn run_lists_models_when_none_selected() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    let output = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "auto",
            "任务",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("MODEL_NOT_SELECTED"), "stderr: {stderr}");
    assert!(stderr.contains("可用模型"), "stderr: {stderr}");
}

#[tokio::test]
async fn chat_requires_tty() {
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    let output = harness(&host_dir(&dir))
        .args(["chat", "-C", source.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TTY"), "stderr: {stderr}");
}

#[tokio::test]
async fn run_forwards_supported_effort_to_model() {
    let base = mock_finished().await;
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    configure_model(&host_dir(&dir), &base).await;
    // 为 fixture 模型补充支持档位。
    let params = format!(
        r#"{{"config":{{"id":"mock","name":"mock","providerId":"fixture","apiMode":"chatCompletions","model":"model","credentialRef":"test/none","baseUrl":"{base}","maxOutputTokens":1024,"efforts":["low","high"],"effort":"high"}}}}"#
    );
    let output = harness(&host_dir(&dir))
        .args(["call", "model.save", &params])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "auto",
            "-m",
            "mock",
            "--effort",
            "high",
            "任务",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn run_rejects_effort_outside_model_levels() {
    let base = mock_finished().await;
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    configure_model(&host_dir(&dir), &base).await;
    let output = harness(&host_dir(&dir))
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--approval",
            "auto",
            "-m",
            "mock",
            "--effort",
            "high",
            "任务",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("MODEL_EFFORT"), "stderr: {stderr}");
}

#[tokio::test]
async fn run_resolves_model_effort_and_approval_from_preset() {
    let base = mock_finished().await;
    let dir = tempfile::tempdir().unwrap();
    let source = source_dir(dir.path());
    let host = host_dir(&dir);
    configure_model(&host, &base).await;
    // mock 模型补档位列表（efforts 由 model.save upsert 覆盖）。
    let base_url = base.trim_end_matches("/v1").to_string();
    let effortful = format!(
        r#"{{"config":{{"id":"mock","name":"mock","providerId":"fixture","apiMode":"chatCompletions","model":"model","credentialRef":"test/none","baseUrl":"{base_url}/v1","maxOutputTokens":1024,"efforts":["low","high"]}}}}"#
    );
    for (method, params) in [
        ("model.save", effortful),
        (
            "preset.save".into(),
            r#"{"preset":{"id":"refactor","name":"批量重构","modelId":"mock","effort":"low","approval":"auto"}}"#.to_string(),
        ),
    ] {
        let output = harness(&host)
            .args(["call", method, &params])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{method} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // 非交互式（stdin 非 TTY）下不传 -m 与 --approval：模型、档位与审批策略全部来自预设。
    let output = harness(&host)
        .args([
            "run",
            "-C",
            source.to_str().unwrap(),
            "--json",
            "--preset",
            "refactor",
            "总结项目",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("finished"), "stdout: {stdout}");

    // preset.list 经 JSON-RPC 可读，含内置与用户预设。
    let output = harness(&host)
        .args(["call", "preset.list", "{}"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let listing: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("preset.list 返回 JSON");
    let ids: Vec<&str> = listing
        .as_array()
        .unwrap()
        .iter()
        .map(|preset| preset["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["builtin:default", "refactor"]);
}
