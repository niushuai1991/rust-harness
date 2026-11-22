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
    let info = manager
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    (tmp, manager, info)
}
/// worktree 模式夹具：git 初始化源目录后创建工作区并登记会话 worktree。
async fn worktree_workspace() -> (tempfile::TempDir, WorkspaceManager, WorkspaceInfo, String) {
    let (tmp, manager, mut info) = workspace().await;
    for args in [
        vec!["init", "-q"],
        vec!["-c", "core.hooksPath=NUL", "add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "NUL")
                .arg("-C")
                .arg(&info.source_path)
                .args(&args)
                .status()
                .unwrap()
                .success(),
            "git fixture 失败"
        );
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    let worktree = manager
        .ensure_session_worktree(&info.id, &session_id)
        .unwrap();
    info.worktrees.insert(session_id.clone(), worktree);
    (tmp, manager, info, session_id)
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
                        &info.id,
                        CancellationToken::new(),
                        true,
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
/// 进程存活时 try_exit_code 必须立即返回 None（terminal_write 依赖该语义逐条返回），
/// 退出后返回退出码——阻塞式 await 在存活期会永久挂起（PR #172 真实抽查暴露）。
#[tokio::test]
async fn pty_try_exit_code_is_nonblocking_while_running() {
    skip_without_conpty!();
    let (_tmp, manager, info) = workspace().await;
    let _lease = manager.acquire(&info.id).unwrap();
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.replica_path),
        &info.id,
        CancellationToken::new(),
        true,
    )
    .await
    .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        pty.try_exit_code(),
        None,
        "存活的终端必须立即返回 None 而不是阻塞"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "try_exit_code 不得阻塞"
    );
    pty.write_input("exit\r\n").await.unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let code = loop {
        if let Some(code) = pty.try_exit_code() {
            break code;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "退出码未在 15 秒内可得"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(code, 0);
}
#[tokio::test]
async fn pty_roundtrip_resize_exit_and_lease_lifecycle() {
    skip_without_conpty!();
    let (_tmp, manager, info) = workspace().await;
    // 租约由调用方（生产中为 terminal_open）持有：测试层显式 acquire。
    let lease = manager.acquire(&info.id).unwrap();
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.replica_path),
        &info.id,
        CancellationToken::new(),
        true,
    )
    .await
    .unwrap();
    // 终端存续期间共享租约被持有：人工同步（独占 refresh）被拒。
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
    drop(lease);
    manager.refresh(&info.id).unwrap();
}
#[tokio::test]
async fn pty_terminate_kills_process_tree_and_releases_lease() {
    skip_without_conpty!();
    let (_tmp, manager, info) = workspace().await;
    let lease = manager.acquire(&info.id).unwrap();
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.replica_path),
        &info.id,
        CancellationToken::new(),
        true,
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
    drop(lease);
    manager.refresh(&info.id).unwrap();
}
/// worktree 会话终端：在会话 worktree 内交互（受限令牌，capability=会话 id）。
#[tokio::test]
async fn pty_worktree_session_runs_in_session_worktree() {
    skip_without_conpty!();
    let (_tmp, manager, info, session_id) = worktree_workspace().await;
    let entry = info.worktrees.get(&session_id).unwrap();
    let lease = manager.acquire(&info.id).unwrap();
    let mut pty = PtyService::spawn(
        std::path::Path::new(&entry.path),
        &session_id,
        CancellationToken::new(),
        true,
    )
    .await
    .unwrap();
    pty.write_input(&format!(
        "[IO.File]::WriteAllText('wt-pty.txt','{MARKER}'); Write-Output done\r\n"
    ))
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let marker_path = PathBuf::from(&entry.path).join("wt-pty.txt");
    while !marker_path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "worktree 标记文件未在 30 秒内出现"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(
        fs::read_to_string(&marker_path).unwrap().trim(),
        MARKER,
        "终端应写入会话 worktree"
    );
    assert!(
        !PathBuf::from(&info.source_path).join("wt-pty.txt").exists(),
        "不得写入源目录"
    );
    drop(pty);
    drop(lease);
}
/// 无项目会话终端：scratch 根内交互（受限令牌，capability=会话 id，无工作区租约）。
#[tokio::test]
async fn pty_scratch_session_runs_in_scratch_root() {
    skip_without_conpty!();
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::tempdir_in(base).unwrap();
    let manager = WorkspaceManager::new(tmp.path().join("host")).unwrap();
    let session_id = uuid::Uuid::new_v4().to_string();
    let root = manager.ensure_scratch_root(&session_id).unwrap();
    let mut pty = PtyService::spawn(
        std::path::Path::new(&root),
        &session_id,
        CancellationToken::new(),
        true,
    )
    .await
    .unwrap();
    pty.write_input(&format!(
        "[IO.File]::WriteAllText('scratch-pty.txt','{MARKER}'); Write-Output done\r\n"
    ))
    .await
    .unwrap();
    let marker_path = root.join("scratch-pty.txt");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !marker_path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "scratch 标记文件未在 30 秒内出现"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(fs::read_to_string(&marker_path).unwrap().trim(), MARKER);
    drop(pty);
}
/// 直接模式终端：宿主令牌（无沙箱）在源项目目录交互。
#[tokio::test]
async fn pty_direct_session_runs_unsandboxed_in_source() {
    skip_without_conpty!();
    let (_tmp, _manager, info) = workspace().await;
    // Replica 夹具的源目录同样满足 direct 语义：root=源目录、无沙箱授权。
    let mut pty = PtyService::spawn(
        std::path::Path::new(&info.source_path),
        "",
        CancellationToken::new(),
        false,
    )
    .await
    .unwrap();
    pty.write_input(&format!(
        "[IO.File]::WriteAllText('direct-pty.txt','{MARKER}'); Write-Output done\r\n"
    ))
    .await
    .unwrap();
    let marker_path = PathBuf::from(&info.source_path).join("direct-pty.txt");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while !marker_path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "direct 标记文件未在 30 秒内出现"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(fs::read_to_string(&marker_path).unwrap().trim(), MARKER);
    drop(pty);
}
