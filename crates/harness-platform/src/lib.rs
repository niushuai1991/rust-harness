//! Host workspace and process isolation services.
pub mod diffview;
mod paths;
mod pty;
mod secrets;
mod shell;
mod workspace;
pub use paths::{
    list_files, read_file, read_file_bounded, read_file_bounded_sized, resolve_path, write_file,
};
pub use pty::{OutputBuffer, PtyService, READ_CAP, SandboxedPty};
pub use secrets::{delete_secret, get_secret, set_secret};
pub use shell::{ProcessOutput, SandboxedStdio, ShellService, classify_sandbox_denial};
pub use workspace::{WorkspaceLease, WorkspaceManager};
pub(crate) fn io(e: impl std::fmt::Display) -> harness_contracts::AppError {
    harness_contracts::AppError::new("IO", e.to_string())
}
