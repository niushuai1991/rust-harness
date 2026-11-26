#[cfg(windows)]
#[tokio::test]
async fn direct_stdio_uses_explicit_working_directory() {
    use harness_platform::ShellService;
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;

    let root = tempfile::tempdir().unwrap();
    let mut child = ShellService::spawn_stdio_in(
        root.path(),
        "",
        "powershell.exe",
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "(Get-Location).Path".into(),
        ],
        CancellationToken::new(),
        false,
    )
    .await
    .unwrap();
    let mut output = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        child.read_to_string(&mut output),
    )
    .await
    .unwrap()
    .unwrap();
    let observed = std::path::Path::new(output.trim());
    assert_eq!(
        dunce::canonicalize(observed).unwrap(),
        dunce::canonicalize(root.path()).unwrap()
    );
}
#[cfg(windows)]
#[tokio::test]
async fn sandboxed_stdio_uses_scratch_capability_and_root() {
    use harness_platform::{ShellService, WorkspaceManager};
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;

    let host = tempfile::tempdir().unwrap();
    let manager = WorkspaceManager::new(host.path().to_path_buf()).unwrap();
    let session = uuid::Uuid::new_v4().to_string();
    let root = manager.ensure_scratch_root(&session).unwrap();
    let mut child = ShellService::spawn_stdio_in(
        &root,
        &session,
        "powershell.exe",
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "(Get-Location).Path".into(),
        ],
        CancellationToken::new(),
        true,
    )
    .await
    .unwrap();
    let mut output = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        child.read_to_string(&mut output),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        dunce::canonicalize(output.trim()).unwrap(),
        dunce::canonicalize(&root).unwrap()
    );
}
#[cfg(windows)]
#[tokio::test]
async fn direct_stdio_does_not_inherit_unrequested_environment() {
    use harness_platform::ShellService;
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;

    let root = tempfile::tempdir().unwrap();
    assert!(std::env::var_os("USERPROFILE").is_some());
    let mut child = ShellService::spawn_stdio_in(
        root.path(),
        "",
        "powershell.exe",
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "[bool](Test-Path Env:USERPROFILE)".into(),
        ],
        CancellationToken::new(),
        false,
    )
    .await
    .unwrap();
    let mut output = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        child.read_to_string(&mut output),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.trim(), "False");
}
#[cfg(windows)]
#[tokio::test]
async fn direct_stdio_passes_explicit_environment_values() {
    use harness_platform::ShellService;
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;
    let root = tempfile::tempdir().unwrap();
    let mut values = std::collections::BTreeMap::new();
    values.insert("MCP_FIXTURE_VALUE".to_owned(), "expected".to_owned());
    let mut child = ShellService::spawn_stdio_with_env(
        root.path(),
        "",
        "powershell.exe",
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "$env:MCP_FIXTURE_VALUE".into(),
        ],
        CancellationToken::new(),
        false,
        &values,
    )
    .await
    .unwrap();
    let mut output = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        child.read_to_string(&mut output),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.trim(), "expected");
}
#[cfg(windows)]
#[tokio::test]
async fn dropping_stdio_terminates_running_process() {
    use harness_platform::ShellService;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio_util::sync::CancellationToken;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let root = tempfile::tempdir().unwrap();
    let child = ShellService::spawn_stdio_in(
        root.path(),
        "",
        "powershell.exe",
        &[
            "-NoProfile".into(),
            "-Command".into(),
            "Write-Output $PID; Start-Sleep -Seconds 30".into(),
        ],
        CancellationToken::new(),
        false,
    )
    .await
    .unwrap();
    let mut reader = BufReader::new(child);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let pid: u32 = line.trim().parse().unwrap();
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null());
    drop(reader);
    let result = unsafe { WaitForSingleObject(handle, 5000) };
    unsafe { CloseHandle(handle) };
    assert_eq!(
        result, WAIT_OBJECT_0,
        "server process should exit when its connection is dropped"
    );
}
