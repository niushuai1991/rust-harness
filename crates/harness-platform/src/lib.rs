//! Host workspace and process isolation services.
mod paths;
mod secrets;
mod shell;
mod workspace;
pub use paths::{list_files, read_file, read_file_bounded, resolve_path, write_file};
pub use secrets::{delete_secret, get_secret, set_secret};
pub use shell::{ProcessOutput, SandboxedStdio, ShellService};
pub use workspace::{WorkspaceLease, WorkspaceManager};
pub(crate) fn io(e: impl std::fmt::Display) -> harness_contracts::AppError {
    harness_contracts::AppError::new("IO", e.to_string())
}
