//! Trusted Rust capabilities use the same guarded execution path as built-ins.
use crate::model::ModelProvider;
use async_trait::async_trait;
use harness_contracts::*;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub workspace_id: String,
    pub root: PathBuf,
    pub provider: Arc<dyn ModelProvider>,
}
#[async_trait]
pub trait ExternalTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn requires_approval(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        context: ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult>;
}
pub(crate) type Tools = Arc<Mutex<BTreeMap<String, Arc<dyn ExternalTool>>>>;
pub struct Registration {
    pub(crate) name: String,
    pub(crate) registry: Weak<Mutex<BTreeMap<String, Arc<dyn ExternalTool>>>>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.lock().unwrap().remove(&self.name);
        }
    }
}
