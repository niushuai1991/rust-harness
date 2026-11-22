//! GUI 宿主形态的 PTY 附着探针（诊断工具，不参与测试）。
//!
//! 背景：真实 ConPTY 集成测试（控制台宿主）附着成功，桌面端（GUI 宿主，`windows_subsystem="windows"`）
//! 启动的直接模式终端却弹出独立控制台窗口——子进程未附着 ConPTY。本探针以完全相同的 GUI 形态
//! 启动终端、写入标记命令并等待输出回流：输出含标记 = 附着成功；无输出 = 附着失败（弹窗复现）。
//! 结果写入系统临时目录 `gui_pty_result.txt`（GUI 程序无控制台输出）。
#![cfg_attr(not(windows), windows_subsystem = "windows")]
#[cfg(windows)]
fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime.block_on(probe());
    let report = match result {
        Ok(seen) => format!("attached_output={seen:?}"),
        Err(e) => format!("error={e}"),
    };
    let path = std::env::temp_dir().join("gui_pty_result.txt");
    std::fs::write(&path, &report).expect("写结果文件");
}

#[cfg(windows)]
async fn probe() -> harness_contracts::AppResult<String> {
    use harness_platform::PtyService;
    use std::time::{Duration, Instant};
    let base = std::env::temp_dir().join(format!("gui_pty_{}", harness_contracts::new_id()));
    std::fs::create_dir_all(&base).expect("建探针目录");
    let mut pty = PtyService::spawn(&base, "", tokio_util::sync::CancellationToken::new(), false)
        .await
        .expect("spawn 终端");
    pty.write_input(
        "Write-Output gui-pty-probe-marker; [IO.File]::WriteAllText('marker.txt','ok')\r\n",
    )
    .await
    .expect("写终端输入");
    let mut seen = String::new();
    let mut exit: Option<i32> = None;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let chunk = tokio::time::timeout(Duration::from_secs(2), pty.read_output(200))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        seen.push_str(&chunk);
        exit = pty.try_exit_code();
        if seen.contains("gui-pty-probe-marker") || exit.is_some() {
            break;
        }
    }
    let marker = base.join("marker.txt").exists();
    Ok(format!(
        "seen={seen:?} exit={exit:?} markerWritten={marker}"
    ))
}

#[cfg(not(windows))]
fn main() {
    println!("Windows only");
}
