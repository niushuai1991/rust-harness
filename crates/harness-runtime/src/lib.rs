//! Agent runtime, independent of the desktop carrier.
pub mod agent;
pub mod chatgpt;
pub mod model;
mod registry;
pub mod request_timing;
mod token_budget;
pub use agent::Runtime;
pub use registry::{ExternalTool, Registration, ToolContext};
pub mod providers;

mod artifacts;
pub mod hook_trust;
mod hooks;

pub mod jobs;
pub mod mcp_config;
pub mod plugin_ui;
pub mod presets;
pub mod skills;
