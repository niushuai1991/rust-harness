//! Human-only declarative plugin entry points. Agent-facing tools continue through agent approval.
use crate::Runtime;
use harness_contracts::{AppError, AppResult, ToolCall, ToolResult, WorkspaceFilePreview, new_id};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;
fn error(e: impl std::fmt::Display) -> AppError {
    AppError::new("PLUGIN_UI", e.to_string())
}
struct Active<'a> {
    runtime: &'a Runtime,
    session: String,
    cancel: CancellationToken,
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.runtime.active.lock().unwrap().remove(&self.session);
    }
}
impl Runtime {
    fn prepare_plugin_workspace(&self, workspace: &str) -> AppResult<()> {
        self.workspaces.get(workspace)?;
        self.plugins.set_workspace_policy(
            workspace,
            [
                "tools",
                "hooks",
                "log",
                "kv",
                "workspace.read",
                "workspace.write",
                "http",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        );
        Ok(())
    }

    pub fn plugin_contributions(&self, workspace: &str) -> AppResult<Vec<Value>> {
        self.prepare_plugin_workspace(workspace)?;
        self.plugins.workspace_contributions(workspace)
    }
    pub async fn plugin_settings_save(
        &self,
        workspace: &str,
        plugin: &str,
        values: Value,
    ) -> AppResult<()> {
        self.prepare_plugin_workspace(workspace)?;
        self.plugins.save_settings(workspace, plugin, values).await
    }
    pub async fn plugin_command(
        &self,
        session: &str,
        plugin: &str,
        name: &str,
        arguments: Value,
    ) -> AppResult<ToolResult> {
        let _boundary = self.session_boundary(session).await;
        let info = self.store.session(session).await?;
        let lease = self.workspaces.acquire(&info.workspace_id)?;
        let declared = self
            .plugin_contributions(&info.workspace_id)?
            .iter()
            .any(|p| {
                p["id"] == plugin
                    && p["commands"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|c| c["name"] == name))
            });
        if !declared {
            return Err(error("command disabled or not granted"));
        }
        let cancel = self.plugins.command_cancellation(plugin)?;
        self.active
            .lock()
            .unwrap()
            .insert(session.into(), cancel.clone());
        let _active = Active {
            runtime: self,
            session: session.into(),
            cancel: cancel.clone(),
        };
        let call = ToolCall {
            id: new_id(),
            name: format!("plugin__{plugin}__command:{name}"),
            arguments: arguments.clone(),
        };
        self.append(
            session,
            "plugin.command.started",
            json!({"callId":call.id,"pluginId":plugin,"name":name,"arguments":arguments}),
        )
        .await?;
        let result = async {
            self.approve(session, &call, &cancel, false, None).await?;
            self.plugins
                .execute_command(
                    &info.workspace_id,
                    Path::new(&lease.info.replica_path),
                    plugin,
                    name,
                    arguments,
                    cancel.clone(),
                )
                .await
        }
        .await;
        match result {
            Ok(output) => {
                self.append(session,"plugin.command.result",json!({"callId":call.id,"pluginId":plugin,"name":name,"content":output.content,"isError":output.is_error,"status":"completed"})).await?;
                Ok(output)
            }
            Err(e) => {
                self.append(session,"plugin.command.result",json!({"callId":call.id,"pluginId":plugin,"name":name,"content":e.to_string(),"isError":true,"status":if cancel.is_cancelled(){"cancelled"}else{"failed"}})).await?;
                Err(e)
            }
        }
    }
    pub fn plugin_file_preview(&self, workspace: &str, path: &str) -> AppResult<String> {
        let info = self.workspaces.get(workspace)?;
        harness_platform::read_file_bounded(Path::new(&info.replica_path), path, 32768)
    }
    /// 桌面端右侧面板的文件预览：32KiB 截断 + 二进制占位语义。
    pub fn workspace_file_preview(
        &self,
        workspace: &str,
        path: &str,
    ) -> AppResult<WorkspaceFilePreview> {
        let info = self.workspaces.get(workspace)?;
        let (content, size, read) =
            harness_platform::read_file_bounded_sized(Path::new(&info.replica_path), path, 32768)?;
        Ok(WorkspaceFilePreview {
            path: path.to_owned(),
            truncated: read < size,
            binary: content.contains('\0'),
            content: if content.contains('\0') {
                String::new()
            } else {
                content
            },
            size_bytes: size,
        })
    }
}
/// Called exclusively from the WASM blocking worker, after the Store capability check.
/// Dropping the async request cancels native I/O; redirects, credentials and oversized bodies fail closed.
pub fn http_get(url: &str, cancel: CancellationToken) -> AppResult<String> {
    let url = reqwest::Url::parse(url).map_err(error)?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(error("invalid plugin HTTP URL"));
    }
    tokio::runtime::Handle::current().block_on(async move {
 let request=async {
 let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).build().map_err(error)?;
 let mut response=client.get(url).send().await.map_err(|_|error("plugin HTTP request failed"))?.error_for_status().map_err(|_|error("plugin HTTP status failed"))?;
 if response.content_length().is_some_and(|n|n>1<<20){return Err(error("plugin HTTP body limit"))}let mut body=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|error("plugin HTTP body failed"))?{if body.len()+chunk.len()>1<<20{return Err(error("plugin HTTP body limit"))}body.extend_from_slice(&chunk);}String::from_utf8(body).map_err(|_|error("plugin HTTP body must be UTF-8"))};
 tokio::select!{biased;_=cancel.cancelled()=>Err(error("plugin HTTP cancelled")),result=tokio::time::timeout(Duration::from_secs(30),request)=>result.map_err(|_|error("plugin HTTP timeout"))?}
 })
}
