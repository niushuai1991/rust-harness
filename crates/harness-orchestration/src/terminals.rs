//! Durable terminal sessions: the execution-world seam and its platform binding.
//!
//! `PtyWorld` is the single seam through which terminals enter an execution
//! world; the local world is the sandboxed ConPTY binding in `harness-platform`.
//! A future remote (SSH) world implements the same traits without touching the
//! tool layer below.
use async_trait::async_trait;
use harness_contracts::*;
use harness_platform::{PtyService, SandboxedPty, WorkspaceLease};
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;
pub(crate) const TERMINAL_LIMIT: usize = 4;
/// Execution-world seam: opening one terminal inside one world.
#[async_trait]
pub trait PtyWorld: Send + Sync {
    async fn spawn(
        &self,
        root: &Path,
        cancel: CancellationToken,
    ) -> AppResult<Box<dyn TerminalSession>>;
}
/// One live terminal: interactive stdin, incremental stdout, exit and teardown.
#[async_trait]
pub trait TerminalSession: Send {
    async fn write_input(&mut self, input: &str) -> AppResult<()>;
    /// Incremental unread output; waits up to `yield_ms` when nothing is pending.
    async fn read_output(&mut self, yield_ms: u64) -> AppResult<String>;
    /// Exit code, `None` while still running.
    async fn exit_code(&mut self) -> Option<i32>;
    /// Spill tail for persistence: (text, truncated-flag).
    fn spill(&self) -> (String, bool);
    fn terminate(&self);
}
/// Local execution world: interactive sandboxed shell in the managed replica.
pub struct PlatformWorld;
#[async_trait]
impl PtyWorld for PlatformWorld {
    async fn spawn(
        &self,
        root: &Path,
        cancel: CancellationToken,
    ) -> AppResult<Box<dyn TerminalSession>> {
        let pty = PtyService::spawn(root, cancel).await?;
        Ok(Box::new(PlatformTerminal { pty }))
    }
}
struct PlatformTerminal {
    pty: SandboxedPty,
}
#[async_trait]
impl TerminalSession for PlatformTerminal {
    async fn write_input(&mut self, input: &str) -> AppResult<()> {
        self.pty.write_input(input).await
    }
    async fn read_output(&mut self, yield_ms: u64) -> AppResult<String> {
        self.pty.read_output(yield_ms).await
    }
    async fn exit_code(&mut self) -> Option<i32> {
        self.pty.exit_code().await
    }
    fn spill(&self) -> (String, bool) {
        self.pty.spill()
    }
    fn terminate(&self) {
        self.pty.terminate();
    }
}
/// A live terminal registered to one session: token kills the tree on shutdown.
pub(crate) struct TerminalEntry {
    pub session_id: String,
    pub session: tokio::sync::Mutex<Box<dyn TerminalSession>>,
    pub _lease: WorkspaceLease,
    pub token: CancellationToken,
}
