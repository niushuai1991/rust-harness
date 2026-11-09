#![cfg(windows)]
use harness_contracts::WorkspaceInfo;
use harness_platform::*;
use std::{
    fs,
    path::PathBuf,
    sync::{OnceLock, mpsc},
};
use tokio_util::sync::CancellationToken;
const MARKER: &str = "pty-e2e-42";
async fn workspace() -> (tempfile::TempDir, WorkspaceManager, WorkspaceInfo) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let info = manager.create(&source, None).unwrap();
    (tmp, manager, info)
}
/// ConPTY 客户端连接预检：部分 Windows 预览版上新建伪控制台的客户端会立即
/// 以 0xC0000142 死亡（管道无任何输出）。不可用时跳过集成测试并打印原因，
/// 真实验证在 CI windows runner 上进行。返回 Some(原因) 表示不可用。
fn conpty_unavailable_reason() -> Option<String> {
    static REASON: OnceLock<Option<String>> = OnceLock::new();
    REASON
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let result = runtime.block_on(async {
                    let (_tmp, _manager, info) = workspace().await;
                    let Ok(mut pty) = PtyService::spawn(
                        std::path::Path::new(&info.replica_path),
                        CancellationToken::new(),
                    )
                    .await else {
                        return Some("PtyService::spawn 失败".into());
                    };
                    if pty.write_input("Write-Output 'conpty-preflight'\r\n").await.is_err() {
                        return Some("写入终端输入失败".into());
                    }
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
                    let mut seen = String::new();
                    while !seen.contains("conpty-preflight") {
                        if std::time::Instant::now() >= deadline {
                            return Some("8 秒内未收到任何控制台输出".into());
                        }
                        let chunk = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            pty.read_output(200),
                        )
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .unwrap_or_default();
                        seen.push_str(&chunk);
                        if seen.is_empty()
                            && let Ok(Some(code)) =
                                tokio::time::timeout(std::time::Duration::from_millis(50), pty.exit_code()).await
                        {
                            return Some(format!(
                                "终端子进程立即退出（exit={code:#x}，疑似 0xC0000142 客户端连接失败）"
                            ));
                        }
                    }
                    None
                });
                let _ = tx.send(result);
            });
            rx.recv_timeout(std::time::Duration::from_secs(30))
                .unwrap_or_else(|_| Some("预检超时".into()))
        })
        .clone()
}
/// 在 ConPTY 不可用的机器上跳过测试并说明去向。
macro_rules! skip_without_conpty {
    () => {
        if let Some(reason) = conpty_unavailable_reason() {
            eprintln!(
                "跳过：本机 ConPTY 客户端连接不可用（{reason}）；该测试在 GitHub CI windows runner 上真实运行"
            );
            return;
        }
    };
}
#[tokio::test]
async fn pty_roundtrip_resize_exit_and_lease_lifecycle() {
    skip_without_conpty!();
    let (_tmp, manager, info) = workspace().await;
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.replica_path),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    // 持久终端存续期间共享租约被持有：人工同步（独占 refresh）被拒。
    assert_eq!(manager.refresh(&info.id).unwrap_err().code, "BUSY");
    pty.resize(80, 24).unwrap();
    pty.write_input(&format!("$sum=40+2; Write-Output '{MARKER}'\r\n"))
        .await
        .unwrap();
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !seen.contains(MARKER) {
        if tokio::time::Instant::now() >= deadline {
            panic!("marker not seen within 30s; output so far: {seen:?}");
        }
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), pty.read_output(500))
            .await
            .unwrap()
            .unwrap();
        seen.push_str(&chunk);
    }
    assert!(
        !seen.contains("\x1b"),
        "model-facing output must be ANSI-stripped: {seen:?}"
    );
    pty.write_input("exit\r\n").await.unwrap();
    let code = tokio::time::timeout(std::time::Duration::from_secs(15), pty.exit_code())
        .await
        .unwrap();
    assert_eq!(code, Some(0));
    let (tail, _truncated) = pty.spill();
    assert!(tail.contains(MARKER), "spill tail must keep output: {tail}");
    drop(pty);
    manager.refresh(&info.id).unwrap();
}
#[tokio::test]
async fn pty_terminate_kills_process_tree_and_releases_lease() {
    skip_without_conpty!();
    let (_tmp, manager, info) = workspace().await;
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.replica_path),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let ps = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    pty.write_input(&format!(
        "$child=Start-Process -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 60\"' -PassThru; [IO.File]::WriteAllText('child.pid',[string]$child.Id)",
        ps.display()
    ))
    .await
    .unwrap();
    let pid_path = PathBuf::from(&info.replica_path).join("child.pid");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline && !pid_path.exists() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(pid_path.exists(), "descendant did not start");
    let pid = fs::read_to_string(&pid_path)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    pty.terminate();
    let code = tokio::time::timeout(std::time::Duration::from_secs(15), pty.exit_code())
        .await
        .unwrap();
    assert_ne!(code, Some(0), "terminated shell must not exit cleanly");
    unsafe {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let child = OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if !child.is_null() {
            assert_eq!(
                WaitForSingleObject(child, 5000),
                WAIT_OBJECT_0,
                "descendant must terminate"
            );
            CloseHandle(child);
        }
    }
    drop(pty);
    manager.refresh(&info.id).unwrap();
}
