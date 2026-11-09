//! Agent runtime, independent of the desktop carrier.
pub mod agent;
pub mod chatgpt;
pub mod model;
mod registry;
mod token_budget;
pub use agent::Runtime;
pub use registry::{ExternalTool, Registration, ToolContext};
pub mod providers;

mod artifacts;

pub mod plugin_ui;
pub mod presets;
pub mod skills;
