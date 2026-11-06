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
