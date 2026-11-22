//! GUI 宿主下直接模式 ConPTY 的真实命令输出检查。
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use std::{path::PathBuf, time::Duration};

    let mut args = std::env::args().skip(1);
    let mut report = None;
    let mut work_dir = None;
    let mut command =
        "Write-Output ('gui-pty-' + 'probe-marker'); [IO.File]::WriteAllText('marker.txt','ok')"
            .to_owned();
    let mut timeout = Duration::from_secs(15);
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--report", Some(value)) => report = Some(PathBuf::from(value)),
            ("--work-dir", Some(value)) => work_dir = Some(PathBuf::from(value)),
            ("--command", Some(value)) => command = value,
            ("--timeout-ms", Some(value)) => match value.parse::<u64>() {
                Ok(ms) if ms > 0 => timeout = Duration::from_millis(ms),
                _ => std::process::exit(2),
            },
            _ => std::process::exit(2),
        }
    }
    let Some(report) = report else {
        std::process::exit(2);
    };
    let Some(work_dir) = work_dir else {
        std::process::exit(2);
    };
    let source = harness_platform::PtyService::conpty_source();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))
        .and_then(|runtime| runtime.block_on(probe(&work_dir, &command, timeout)));
    let (message, code) = match result {
        Ok(output) => (format!("PASS source={source} output={output:?}"), 0),
        Err(error) => (format!("FAIL source={source} {error}"), 1),
    };
    if std::fs::write(report, message).is_err() {
        std::process::exit(2);
    }
    std::process::exit(code);
}

#[cfg(windows)]
async fn probe(
    work_dir: &std::path::Path,
    command: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use harness_platform::PtyService;
    use tokio::time::{Instant, timeout_at};

    let root = tempfile::tempdir_in(work_dir).map_err(|e| format!("temporary directory: {e}"))?;
    let deadline = Instant::now() + timeout;
    let mut pty = timeout_at(
        deadline,
        PtyService::spawn(
            root.path(),
            "",
            tokio_util::sync::CancellationToken::new(),
            false,
        ),
    )
    .await
    .map_err(|_| "timeout spawning terminal".to_owned())?
    .map_err(|e| format!("spawn terminal: {e}"))?;
    let input = format!("{}\r\n", command.replace('\n', "\r\n"));
    timeout_at(deadline, pty.write_input(&input))
        .await
        .map_err(|_| "timeout writing command".to_owned())?
        .map_err(|e| format!("write command: {e}"))?;
    let mut output = String::new();
    loop {
        let marker = std::fs::read_to_string(root.path().join("marker.txt"));
        if output.contains("gui-pty-probe-marker") && matches!(marker, Ok(ref s) if s == "ok") {
            return Ok(output);
        }
        if let Some(code) = pty.try_exit_code() {
            return Err(format!(
                "terminal exited early: {code}; output={output:?}; marker={marker:?}"
            ));
        }
        let chunk = timeout_at(deadline, pty.read_output(200))
            .await
            .map_err(|_| {
                format!(
                    "timeout waiting for output and marker; output={output:?}; marker={marker:?}"
                )
            })?
            .map_err(|e| format!("read output: {e}; output={output:?}; marker={marker:?}"))?;
        output.push_str(&chunk);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
    std::process::exit(1);
}
