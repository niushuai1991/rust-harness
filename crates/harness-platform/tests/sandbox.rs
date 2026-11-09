#![cfg(windows)]
use harness_platform::*;
use std::{fs, path::PathBuf};
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn restricted_process_writes_inside_and_cannot_write_outside() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("outside.txt"), "safe").unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let info = manager
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let command = format!(
        "[IO.File]::WriteAllText('inside.txt','ok'); try {{ [IO.File]::WriteAllText('{}','escape'); exit 9 }} catch {{ exit 0 }}",
        source.join("outside.txt").display()
    );
    let out = ShellService::run(
        std::path::Path::new(&info.replica_path),
        &command,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(out.exit_code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        fs::read_to_string(PathBuf::from(info.replica_path).join("inside.txt")).unwrap(),
        "ok"
    );
    assert_eq!(
        fs::read_to_string(source.join("outside.txt")).unwrap(),
        "safe"
    );
}
#[tokio::test]
async fn cancellation_returns_and_releases_runtime_lease() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let info = manager
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let cancel = CancellationToken::new();
    let ps = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let command = format!(
        "$child=Start-Process -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 60\"' -PassThru; [IO.File]::WriteAllText('child.pid',[string]$child.Id); Start-Sleep -Seconds 60",
        ps.display()
    );
    let token = cancel.clone();
    let replica = info.replica_path.clone();
    tokio::spawn(async move {
        // 固定延时在 CI 慢机上可能早于 Start-Process 写出 child.pid，导致取消后
        // 读 pid 文件 NotFound；改为轮询等待 pid 文件出现再取消（上限 10 秒）。
        let pid = PathBuf::from(&replica).join("child.pid");
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline && !pid.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        token.cancel();
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        ShellService::run(std::path::Path::new(&info.replica_path), &command, cancel),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    let pid = fs::read_to_string(PathBuf::from(&info.replica_path).join("child.pid"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
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
    manager.refresh(&info.id).unwrap();
}

#[tokio::test]
async fn sandboxed_stdio_roundtrip_and_drop_releases_lease() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let info = manager
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let ps = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let args = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$line=[Console]::ReadLine(); [Console]::WriteLine($line); Start-Sleep -Seconds 60",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    let process = ShellService::spawn_stdio(
        std::path::Path::new(&info.replica_path),
        &ps.to_string_lossy(),
        &args,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let (mut reader, mut writer) = tokio::io::split(process);
    writer.write_all(b"roundtrip\n").await.unwrap();
    writer.flush().await.unwrap();
    let mut result = vec![0; 11];
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        reader.read_exact(&mut result),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&result, b"roundtrip\r\n");
    assert_eq!(manager.refresh(&info.id).unwrap_err().code, "BUSY");
    drop(reader);
    assert_eq!(manager.refresh(&info.id).unwrap_err().code, "BUSY");
    drop(writer);
    manager.refresh(&info.id).unwrap();
}

#[tokio::test]
async fn run_in_unsandboxed_writes_outside_replica() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("outside.txt"), "safe").unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let info = manager
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    // 提权重跑：宿主令牌直接写源项目（受限令牌下被拦截的位置）。
    let command = format!(
        "[IO.File]::WriteAllText('{}','escalated')",
        source.join("outside.txt").display()
    );
    let out = ShellService::run_in(
        std::path::Path::new(&info.replica_path),
        &info.id,
        &command,
        CancellationToken::new(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(out.exit_code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        fs::read_to_string(source.join("outside.txt")).unwrap(),
        "escalated"
    );
}
