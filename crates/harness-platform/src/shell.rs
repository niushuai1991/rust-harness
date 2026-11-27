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
    #[cfg(windows)]
    pub(crate) process: std::sync::Arc<windows::Handle>,
    #[cfg(windows)]
    pub(crate) stderr_done: tokio::sync::oneshot::Receiver<()>,
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
    /// 执行同步 Hook；持有调用方的工作区租约，取消或超时时清理整个进程树。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_hook_in(
        root: &Path,
        capability_id: &str,
        program: &str,
        args: &[String],
        input: &[u8],
        cancel: CancellationToken,
        sandboxed: bool,
        timeout: std::time::Duration,
    ) -> AppResult<ProcessOutput> {
        #[cfg(windows)]
        {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            const LIMIT: usize = 32 * 1024;
            let child = Self::spawn_stdio_in(
                root,
                capability_id,
                program,
                args,
                cancel.clone(),
                sandboxed,
            )
            .await?;
            let SandboxedStdio {
                mut reader,
                mut writer,
                stderr,
                _guard,
                process,
                stderr_done,
            } = child;
            let stdout_task = tokio::spawn(async move {
                let mut out = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let count = reader.read(&mut chunk).await?;
                    if count == 0 {
                        break;
                    }
                    let take = count.min(LIMIT.saturating_sub(out.len()));
                    out.extend_from_slice(&chunk[..take]);
                }
                Ok::<_, std::io::Error>(out)
            });
            let input = input.to_vec();
            let input_task = tokio::spawn(async move {
                let result = async {
                    writer.write_all(&input).await?;
                    writer.flush().await
                }
                .await;
                drop(writer);
                result
            });
            let wait_task = tokio::task::spawn_blocking(move || windows::wait_process(&process));
            let outcome = tokio::select! {
                _ = cancel.cancelled() => Err(AppError::new("CANCELLED", "Hook 已取消")),
                result = tokio::time::timeout(timeout, wait_task) => match result {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => Err(crate::io(error)),
                    Err(_) => Err(AppError::new("HOOK_TIMEOUT", "Hook 执行超时")),
                },
            };
            // 关闭 Job Object，清理仍持有管道的后代进程；确保读取任务收敛。
            drop(_guard);
            let input_result = input_task.await.map_err(crate::io)?;
            let stdout = stdout_task.await.map_err(crate::io)?.map_err(crate::io)?;
            let _ = stderr_done.await;
            let code = outcome?;
            if code != 2 {
                input_result.map_err(crate::io)?;
            }
            let err = stderr.lock().map_err(crate::io)?;
            Ok(ProcessOutput {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&err[..err.len().min(LIMIT)]).into_owned(),
                exit_code: code,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (
                root,
                capability_id,
                program,
                args,
                input,
                cancel,
                sandboxed,
                timeout,
            );
            Err(AppError::new(
                "UNSUPPORTED",
                "System process isolation is Windows only",
            ))
        }
    }
    /// 在指定执行根执行命令（调用方须已持有工作区运行租约）。
    ///
    /// `capability_id` 派生受限令牌的 capability SID：副本模式为工作区 id，
    /// worktree 模式为会话 id（与 `shell::prepare` 打授权时一致）。
    /// `sandboxed=false` 为提权重跑或直接模式：宿主令牌直接执行。
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
    pub async fn spawn_stdio_in(
        root: &Path,
        capability_id: &str,
        program: &str,
        args: &[String],
        cancel: CancellationToken,
        sandboxed: bool,
    ) -> AppResult<SandboxedStdio> {
        Self::spawn_stdio_with_env(
            root,
            capability_id,
            program,
            args,
            cancel,
            sandboxed,
            &Default::default(),
        )
        .await
    }
    pub async fn spawn_stdio_with_env(
        root: &Path,
        capability_id: &str,
        program: &str,
        args: &[String],
        cancel: CancellationToken,
        sandboxed: bool,
        env: &std::collections::BTreeMap<String, String>,
    ) -> AppResult<SandboxedStdio> {
        #[cfg(windows)]
        {
            windows::spawn_stdio_in(root, capability_id, program, args, cancel, sandboxed, env)
        }
        #[cfg(not(windows))]
        {
            let _ = (root, capability_id, program, args, cancel, sandboxed, env);
            Err(AppError::new(
                "UNSUPPORTED",
                "System process isolation is Windows only",
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
