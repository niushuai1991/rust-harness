use harness_contracts::{AppError, AppResult};
use std::path::Path;
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
#[derive(Default, Clone)]
pub struct ShellService;
pub struct SandboxedStdio {
    pub reader: tokio::fs::File,
    pub writer: tokio::fs::File,
    pub stderr: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    pub(crate) _guard: Box<dyn Send>,
}
impl tokio::io::AsyncRead for SandboxedStdio {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}
impl tokio::io::AsyncWrite for SandboxedStdio {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.writer).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.writer).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}
impl ShellService {
    pub fn new() -> Self {
        Self
    }
    pub async fn run(
        root: &Path,
        command: &str,
        cancel: CancellationToken,
    ) -> AppResult<ProcessOutput> {
        let root = root.to_path_buf();
        let command = command.to_owned();
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(move || windows::run(&root, &command, cancel))
                .await
                .map_err(crate::io)?
        }
        #[cfg(not(windows))]
        {
            let _ = (root, command, cancel);
            Err(AppError::new(
                "UNSUPPORTED",
                "System shell isolation is Windows only",
            ))
        }
    }
    /// 在指定执行根执行命令（调用方须已持有工作区运行租约）。
    ///
    /// `capability_id` 派生受限令牌的 capability SID：副本模式为工作区 id，
    /// worktree 模式为会话 id（与 `shell::prepare` 打授权时一致）。
    /// `sandboxed=false` 为提权重跑：宿主令牌直接执行，一次性的显式用户授权。
    pub async fn run_in(
        root: &Path,
        capability_id: &str,
        command: &str,
        cancel: CancellationToken,
        sandboxed: bool,
    ) -> AppResult<ProcessOutput> {
        let root = root.to_path_buf();
        let capability_id = capability_id.to_owned();
        let command = command.to_owned();
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(move || {
                windows::run_in(&root, &capability_id, &command, cancel, sandboxed)
            })
            .await
            .map_err(crate::io)?
        }
        #[cfg(not(windows))]
        {
            let _ = (root, capability_id, command, cancel, sandboxed);
            Err(AppError::new(
                "UNSUPPORTED",
                "System shell isolation is Windows only",
            ))
        }
    }
    pub async fn spawn_stdio(
        root: &Path,
        program: &str,
        args: &[String],
        cancel: CancellationToken,
    ) -> AppResult<SandboxedStdio> {
        #[cfg(windows)]
        {
            windows::spawn_stdio(root, program, args, cancel)
        }
        #[cfg(not(windows))]
        {
            let _ = (root, program, args, cancel);
            Err(AppError::new(
                "UNSUPPORTED",
                "System process isolation is Windows only",
            ))
        }
    }
}
#[cfg(windows)]
pub(crate) mod windows;
pub(crate) fn prepare(root: &Path, id: &str) -> AppResult<()> {
    #[cfg(windows)]
    {
        windows::prepare(root, id)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, id);
        Ok(())
    }
}

/// 沙箱拒绝归类：命令在受限令牌下失败且输出命中关键词时返回规则名，供提权重跑审批使用。
///
/// 关键词表保持保守（宁漏勿误）：漏判走普通失败呈现，误判会产生多余审批弹窗。
/// 中文环境的关键输出（如「拒绝访问」）一并覆盖。
pub fn classify_sandbox_denial(output: &ProcessOutput) -> Option<&'static str> {
    if output.exit_code == 0 {
        return None;
    }
    const RULES: &[(&str, &str)] = &[
        ("permission denied", "permission-denied"),
        ("access is denied", "access-denied"),
        ("access denied", "access-denied"),
        // PowerShell 对拒绝写的措辞是「Access to the path ... is denied」。
        ("is denied", "access-denied"),
        // 中文 Windows PowerShell 5：对路径"…"的访问被拒绝。
        ("的访问被拒绝", "access-denied"),
        ("拒绝访问", "access-denied"),
        ("operation not permitted", "operation-not-permitted"),
        ("required privilege is not held", "privilege-not-held"),
        ("read-only file system", "read-only-file-system"),
        ("unable to create directory", "unable-to-create-directory"),
        ("cannot create directory", "unable-to-create-directory"),
        ("cannot lock ref", "cannot-lock-ref"),
    ];
    let haystack = format!("{}\n{}", output.stdout, output.stderr).to_lowercase();
    RULES
        .iter()
        .find(|(needle, _)| haystack.contains(needle))
        .map(|(_, rule)| *rule)
}

#[cfg(test)]
mod classify_tests {
    use super::*;

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
        }
    }

    #[test]
    fn failed_git_ref_creation_is_classified() {
        let out = output(
            128,
            "",
            "Preparing worktree (new branch 'feat/x')\nfatal: cannot lock ref 'refs/heads/feat/x': unable to create directory for .git/refs/heads/feat/x\n",
        );
        assert_eq!(
            classify_sandbox_denial(&out),
            Some("unable-to-create-directory")
        );
    }

    #[test]
    fn access_denied_in_either_language_is_classified() {
        assert_eq!(
            classify_sandbox_denial(&output(1, "", "Access is denied.\r\n")),
            Some("access-denied")
        );
        assert_eq!(
            classify_sandbox_denial(&output(1, "", "拒绝访问。\r\n")),
            Some("access-denied")
        );
    }

    #[test]
    fn ordinary_failures_are_not_classified() {
        assert_eq!(
            classify_sandbox_denial(&output(101, "", "error: could not compile `harness`")),
            None
        );
        assert_eq!(
            classify_sandbox_denial(&output(0, "permission denied echo", "")),
            None
        );
    }
}
