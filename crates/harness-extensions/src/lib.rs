//! Installed immutable plugin versions and isolated component invocations.
mod declarative;
pub mod lsp;
mod manifest;
pub mod mcp;
mod package;
mod wasm;
use harness_contracts::{AppResult, PluginInfo, ToolDefinition, ToolResult};
pub use manifest::{Command, Contributions, Manifest, Tool};
use manifest::{fail, identifier};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
pub use wasm::{DenyHost, HostPolicy};
#[derive(Clone, Serialize, Deserialize)]
struct Installed {
    manifest: Manifest,
    enabled: bool,
    grants: Vec<String>,
}
struct State {
    plugins: BTreeMap<String, Installed>,
    tokens: HashMap<String, CancellationToken>,
}
pub struct PluginManager {
    dir: PathBuf,
    state: Mutex<State>,
    engine: Arc<wasm::EngineHost>,
    policy: Mutex<Arc<dyn HostPolicy>>,
    workspace_policies: Mutex<HashMap<String, Vec<String>>>,
    serial: tokio::sync::Mutex<()>,
}
impl PluginManager {
    pub fn new(data_dir: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(data_dir.join("versions")).map_err(fail)?;
        let file = data_dir.join("installed.json");
        let plugins: BTreeMap<String, Installed> = if file.exists() {
            serde_json::from_slice(&fs::read(file).map_err(fail)?).map_err(fail)?
        } else {
            BTreeMap::new()
        };
        for (id, p) in &plugins {
            p.manifest.validate()?;
            if id != &p.manifest.id {
                return Err(fail("registry identity mismatch"));
            }
        }
        let tokens = plugins
            .keys()
            .map(|s| (s.clone(), CancellationToken::new()))
            .collect();
        Ok(Self {
            dir: data_dir,
            state: Mutex::new(State { plugins, tokens }),
            engine: Arc::new(wasm::EngineHost::new()?),
            policy: Mutex::new(Arc::new(DenyHost)),
            workspace_policies: Mutex::new(HashMap::new()),
            serial: tokio::sync::Mutex::new(()),
        })
    }
    pub fn set_host_policy(&self, policy: Arc<dyn HostPolicy>) {
        *self.policy.lock().unwrap() = policy;
    }
    /// Explicit workspace ceiling; effective authority = declared ∩ install grants ∩ this ceiling.
    pub fn set_workspace_policy(&self, id: &str, permissions: Vec<String>) {
        self.workspace_policies
            .lock()
            .unwrap()
            .insert(id.into(), permissions);
    }
    pub fn list(&self) -> AppResult<Vec<PluginInfo>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .plugins
            .values()
            .map(info)
            .collect())
    }
    fn save(&self, plugins: &BTreeMap<String, Installed>) -> AppResult<()> {
        let tmp = self
            .dir
            .join(format!("registry-{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&tmp, serde_json::to_vec_pretty(plugins).map_err(fail)?).map_err(fail)?;
        fs::rename(&tmp, self.dir.join("installed.json")).map_err(fail)
    }
    pub fn install(&self, path: &Path, grants: Vec<String>) -> AppResult<PluginInfo> {
        let staging = self.dir.join(format!("staging-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(fail)?;
        let result = (|| {
            package::unpack(path, &staging)?;
            let manifest: Manifest =
                toml::from_str(&fs::read_to_string(staging.join("plugin.toml")).map_err(fail)?)
                    .map_err(fail)?;
            manifest.validate()?;
            if grants.iter().any(|p| !manifest.permissions.contains(p)) {
                return Err(fail("grant is not declared"));
            }
            if let Some(component) = &manifest.component {
                let relative = package::relative(component)?;
                self.engine.compile(&staging.join(relative))?;
            }
            let mut state = self.state.lock().unwrap();
            if state.plugins.get(&manifest.id).is_some_and(|p| p.enabled) {
                return Err(fail("disable plugin and its dependents before upgrading"));
            }
            for (id, req) in &manifest.dependencies {
                let dependency = state
                    .plugins
                    .get(id)
                    .filter(|p| p.enabled)
                    .ok_or_else(|| fail(format!("missing enabled dependency {id}")))?;
                if !semver::VersionReq::parse(req)
                    .map_err(fail)?
                    .matches(&semver::Version::parse(&dependency.manifest.version).map_err(fail)?)
                {
                    return Err(fail("dependency version mismatch"));
                }
            }
            let parent = self.dir.join("versions").join(&manifest.id);
            fs::create_dir_all(&parent).map_err(fail)?;
            let version = parent.join(&manifest.version);
            if version.exists() {
                return Err(fail("installed version is immutable"));
            }
            // Refuse upgrades that would break an enabled dependent.
            for p in state.plugins.values().filter(|p| p.enabled) {
                if let Some(req) = p.manifest.dependencies.get(&manifest.id)
                    && !semver::VersionReq::parse(req)
                        .map_err(fail)?
                        .matches(&semver::Version::parse(&manifest.version).map_err(fail)?)
                {
                    return Err(fail("upgrade breaks dependent"));
                }
            }
            fs::rename(&staging, &version).map_err(fail)?;
            let installed = Installed {
                manifest,
                enabled: false,
                grants,
            };
            let mut next = state.plugins.clone();
            next.insert(installed.manifest.id.clone(), installed.clone());
            if let Err(e) = self.save(&next) {
                let _ = fs::remove_dir_all(&version);
                return Err(e);
            }
            if let Some(old) = state.tokens.remove(&installed.manifest.id) {
                old.cancel();
            }
            state
                .tokens
                .insert(installed.manifest.id.clone(), CancellationToken::new());
            state.plugins = next;
            Ok(info(&installed))
        })();
        if staging.exists() {
            let _ = fs::remove_dir_all(staging);
        }
        result
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> AppResult<()> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.plugins.clone();
        let p = next.get(id).ok_or_else(|| fail("plugin not found"))?;
        if enabled {
            for (dep, req) in &p.manifest.dependencies {
                let installed = next
                    .get(dep)
                    .filter(|p| p.enabled)
                    .ok_or_else(|| fail("dependency disabled"))?;
                if !semver::VersionReq::parse(req)
                    .map_err(fail)?
                    .matches(&semver::Version::parse(&installed.manifest.version).map_err(fail)?)
                {
                    return Err(fail("dependency mismatch"));
                }
            }
        } else if next
            .values()
            .any(|p| p.enabled && p.manifest.dependencies.contains_key(id))
        {
            return Err(fail("enabled dependent exists"));
        }
        next.get_mut(id).unwrap().enabled = enabled;
        self.save(&next)?;
        if let Some(token) = state.tokens.get(id) {
            token.cancel();
        }
        state.tokens.insert(id.into(), CancellationToken::new());
        state.plugins = next;
        Ok(())
    }
    pub fn remove(&self, id: &str) -> AppResult<()> {
        if !identifier(id) {
            return Err(fail("invalid id"));
        }
        let mut state = self.state.lock().unwrap();
        if state
            .plugins
            .values()
            .any(|p| p.manifest.dependencies.contains_key(id))
        {
            return Err(fail("installed dependent exists"));
        }
        if !state.plugins.contains_key(id) {
            return Err(fail("plugin not found"));
        }
        let mut next = state.plugins.clone();
        next.remove(id);
        self.save(&next)?;
        if let Some(t) = state.tokens.remove(id) {
            t.cancel();
        }
        state.plugins = next;
        // Version payloads are retained until explicit data cleanup; in-flight component readers remain safe.
        Ok(())
    }
    pub fn contributions(&self) -> AppResult<BTreeMap<String, Contributions>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|(_, p)| p.enabled)
            .map(|(id, p)| (id.clone(), p.manifest.contributions.clone()))
            .collect())
    }
    pub fn tool_definitions(&self, workspace_id: &str) -> AppResult<Vec<ToolDefinition>> {
        let policies = self.workspace_policies.lock().unwrap();
        let allowed = policies
            .get(workspace_id)
            .is_some_and(|p| p.iter().any(|s| s == "tools"));
        Ok(self
            .state
            .lock()
            .unwrap()
            .plugins
            .values()
            .filter(|p| allowed && p.enabled && p.grants.iter().any(|s| s == "tools"))
            .flat_map(|p| {
                p.manifest
                    .contributions
                    .tools
                    .iter()
                    .map(move |t| ToolDefinition {
                        name: format!("plugin__{}__{}", p.manifest.id, t.name),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    })
            })
            .collect())
    }
    pub async fn run_hook(
        &self,
        workspace_id: &str,
        root: &Path,
        hook: &str,
        payload: Value,
        cancel: CancellationToken,
    ) -> AppResult<Vec<Value>> {
        if !["before_request", "before_tool", "after_tool"].contains(&hook) {
            return Err(fail("unknown hook"));
        }
        let ids: Vec<String> = self
            .state
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|(_, p)| p.enabled && p.manifest.contributions.hooks.iter().any(|h| h == hook))
            .map(|(id, _)| id.clone())
            .collect();
        let mut outputs = Vec::new();
        for id in ids {
            let result = self
                .execute_invocation(
                    workspace_id,
                    root,
                    &format!("plugin__{id}__hook:{hook}"),
                    payload.clone(),
                    cancel.clone(),
                    false,
                )
                .await?;
            let value: Value = serde_json::from_str(&result.content).map_err(fail)?;
            let valid = match hook {
                "before_request" => value.get("context").is_some_and(Value::is_string),
                "before_tool" => {
                    value.get("name").is_some_and(Value::is_string)
                        && value.get("arguments").is_some_and(Value::is_object)
                        && value.get("id").is_none()
                }
                "after_tool" => serde_json::from_value::<ToolResult>(value.clone()).is_ok(),
                _ => false,
            };
            if !valid {
                return Err(fail("invalid plugin hook output"));
            }
            outputs.push(value);
        }
        Ok(outputs)
    }
    pub async fn execute_tool(
        &self,
        workspace_id: &str,
        root: &Path,
        name: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        if name.contains("__hook:") || name.contains("__command:") {
            return Err(fail(
                "hooks and commands require their explicit entry points",
            ));
        }
        self.execute_invocation(workspace_id, root, name, args, cancel, true)
            .await
    }
    async fn execute_invocation(
        &self,
        workspace_id: &str,
        root: &Path,
        name: &str,
        args: Value,
        cancel: CancellationToken,
        approved_execution: bool,
    ) -> AppResult<ToolResult> {
        let _serial = tokio::select! {biased; _=cancel.cancelled()=>return Err(fail("cancelled while waiting for plugin execution")), lock=self.serial.lock()=>lock}; // serializes transactional KV commits; each call still owns a fresh Store.
        let (id, tool) = name
            .strip_prefix("plugin__")
            .and_then(|s| s.split_once("__"))
            .ok_or_else(|| fail("invalid plugin tool"))?;
        let (installed, disabled) = {
            let state = self.state.lock().unwrap();
            let p = state
                .plugins
                .get(id)
                .filter(|p| p.enabled)
                .ok_or_else(|| fail("plugin disabled/missing"))?;
            (p.clone(), state.tokens[id].clone())
        };
        if cancel.is_cancelled() || disabled.is_cancelled() {
            return Err(fail("cancelled"));
        }
        let hook = tool.strip_prefix("hook:");
        if let Some(hook) = hook {
            if !installed
                .manifest
                .contributions
                .hooks
                .iter()
                .any(|h| h == hook)
            {
                return Err(fail("hook not declared"));
            }
        } else if let Some(command) = tool.strip_prefix("command:") {
            let command = installed
                .manifest
                .contributions
                .commands
                .iter()
                .find(|c| c.name == command)
                .ok_or_else(|| fail("command not declared"))?;
            declarative::validate_arguments(&command.parameters, &args)?;
        } else if !installed
            .manifest
            .contributions
            .tools
            .iter()
            .any(|t| t.name == tool)
        {
            return Err(fail("tool not declared"));
        }
        let ceiling = self
            .workspace_policies
            .lock()
            .unwrap()
            .get(workspace_id)
            .cloned()
            .unwrap_or_default();
        let grants: Vec<_> = installed
            .grants
            .iter()
            .filter(|p| ceiling.contains(p) && installed.manifest.permissions.contains(p))
            .cloned()
            .collect();
        if !grants.iter().any(|s| s == "tools") {
            return Err(fail("tools permission denied"));
        }
        if hook.is_some() && !grants.iter().any(|p| p == "hooks") {
            return Err(fail("hooks permission denied"));
        }
        let engine = self.engine.clone();
        let component = self.engine.compile(
            &self
                .dir
                .join("versions")
                .join(id)
                .join(&installed.manifest.version)
                .join(
                    installed
                        .manifest
                        .component
                        .as_ref()
                        .ok_or_else(|| fail("no component"))?,
                ),
        )?;
        let kvdir = self.dir.join("kv").join(id);
        fs::create_dir_all(&kvdir).map_err(fail)?;
        // Workspace identifiers are encoded, never treated as paths.
        let key = workspace_id
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if key.len() > 200 {
            return Err(fail("workspace id too long"));
        }
        let kvpath = kvdir.join(format!("{key}.json"));
        let kv = if kvpath.exists() {
            serde_json::from_slice(&fs::read(&kvpath).map_err(fail)?).map_err(fail)?
        } else {
            BTreeMap::new()
        };
        let policy = self.policy.lock().unwrap().clone();
        let workspace = workspace_id.to_owned();
        let root = root.to_owned();
        let tool = tool.to_owned();
        let args = args.to_string();
        let hook_name = hook.map(str::to_owned);
        let stopped = disabled.clone();
        let cancelled = cancel.clone();
        let (content, kv) = tokio::task::spawn_blocking(move || {
            engine.execute(wasm::ExecutionRequest {
                component: &component,
                workspace: &workspace,
                root: &root,
                grants,
                policy,
                kv,
                name: &tool,
                args: &args,
                cancel,
                disabled,
                approved_execution,
            })
        })
        .await
        .map_err(fail)??;
        if let Some(hook) = hook_name {
            let value: Value = serde_json::from_str(&content).map_err(fail)?;
            let valid = match hook.as_str() {
                "before_request" => value.get("context").is_some_and(Value::is_string),
                "before_tool" => {
                    value.get("name").is_some_and(Value::is_string)
                        && value.get("arguments").is_some_and(Value::is_object)
                        && value.get("id").is_none()
                }
                "after_tool" => serde_json::from_value::<ToolResult>(value).is_ok(),
                _ => false,
            };
            if !valid {
                return Err(fail("invalid plugin hook output"));
            }
        }
        let state = self.state.lock().unwrap();
        if stopped.is_cancelled()
            || cancelled.is_cancelled()
            || !state.plugins.get(id).is_some_and(|p| p.enabled)
        {
            return Err(fail("plugin disabled/cancelled during call"));
        }
        let tmp = kvpath.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&tmp, serde_json::to_vec(&kv).map_err(fail)?).map_err(fail)?;
        fs::rename(tmp, kvpath).map_err(fail)?;
        Ok(ToolResult {
            content,
            is_error: false,
            diff: None,
        })
    }
}
fn info(p: &Installed) -> PluginInfo {
    PluginInfo {
        id: p.manifest.id.clone(),
        name: p.manifest.name.clone(),
        version: p.manifest.version.clone(),
        kind: if p.manifest.component.is_some() {
            "wasm"
        } else {
            "declarative"
        }
        .into(),
        enabled: p.enabled,
        permissions: p.grants.clone(),
        status: if p.enabled { "enabled" } else { "disabled" }.into(),
        description: p.manifest.description.clone(),
    }
}
