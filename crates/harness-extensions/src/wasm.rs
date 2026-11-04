//! Component execution with no ambient filesystem, process, network, or environment access.
use crate::manifest::fail;
use harness_contracts::AppResult;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
mod bindings {
    wasmtime::component::bindgen!({world:"plugin",path:"../../wit",imports:{default:trappable}});
}
/// All file/network calls must pass the application's policy implementation. Default denies them.
/// Implementations must enforce replica-only paths, approvals and bounded responses themselves.
pub trait HostPolicy: Send + Sync {
    fn read_file(&self, _workspace: &str, _root: &Path, _path: &str) -> AppResult<String> {
        Err(fail("host read policy unavailable"))
    }
    fn write_file(
        &self,
        _workspace: &str,
        _root: &Path,
        _path: &str,
        _contents: &str,
    ) -> AppResult<()> {
        Err(fail("host write policy unavailable"))
    }
    /// Native I/O must support dropping the request on cancellation. Default fails closed.
    fn http_get_cancelled(
        &self,
        _workspace: &str,
        _url: &str,
        _cancel: CancellationToken,
    ) -> AppResult<String> {
        Err(fail("cancellable HTTP policy unavailable"))
    }
    fn http_get(&self, _workspace: &str, _url: &str) -> AppResult<String> {
        Err(fail("host HTTP policy unavailable"))
    }
}
pub struct DenyHost;
impl HostPolicy for DenyHost {}
pub(crate) struct EngineHost {
    engine: Engine,
    linker: Linker<Data>,
}
pub(crate) struct ExecutionRequest<'a> {
    pub component: &'a Component,
    pub workspace: &'a str,
    pub root: &'a Path,
    pub grants: Vec<String>,
    pub policy: Arc<dyn HostPolicy>,
    pub kv: BTreeMap<String, String>,
    pub name: &'a str,
    pub args: &'a str,
    pub cancel: CancellationToken,
    pub disabled: CancellationToken,
    pub approved_execution: bool,
}
struct Data {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    kv: BTreeMap<String, String>,
    grants: Vec<String>,
    workspace: String,
    root: std::path::PathBuf,
    policy: Arc<dyn HostPolicy>,
    approved: bool,
    cancel: CancellationToken,
    external_cancel: CancellationToken,
    disabled: CancellationToken,
}
impl WasiView for Data {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
impl Data {
    fn require(&self, p: &str) -> wasmtime::Result<()> {
        if self.grants.iter().any(|x| x == p) {
            Ok(())
        } else {
            Err(wasmtime::Error::msg(format!("permission denied: {p}")))
        }
    }
}
impl bindings::rust_harness::plugin::host::Host for Data {
    fn log(&mut self, message: String) -> wasmtime::Result<()> {
        self.require("log")?;
        tracing::info!(workspace=%self.workspace,message=%message.chars().take(4096).collect::<String>(),"plugin log");
        Ok(())
    }
    fn read_file(&mut self, path: String) -> wasmtime::Result<Result<String, String>> {
        self.require("workspace.read")?;
        Ok(self
            .policy
            .read_file(&self.workspace, &self.root, &path)
            .map_err(|e| e.to_string())
            .and_then(bounded))
    }
    fn write_file(
        &mut self,
        path: String,
        contents: String,
    ) -> wasmtime::Result<Result<(), String>> {
        self.require("workspace.write")?;
        if !self.approved
            || self.cancel.is_cancelled()
            || self.external_cancel.is_cancelled()
            || self.disabled.is_cancelled()
        {
            return Ok(Err("side effects require approved execution".into()));
        }
        if contents.len() > 1 << 20 {
            return Ok(Err("write limit".into()));
        }
        Ok(self
            .policy
            .write_file(&self.workspace, &self.root, &path, &contents)
            .map_err(|e| e.to_string()))
    }
    fn http_get(&mut self, url: String) -> wasmtime::Result<Result<String, String>> {
        self.require("http")?;
        if !self.approved
            || self.cancel.is_cancelled()
            || self.external_cancel.is_cancelled()
            || self.disabled.is_cancelled()
        {
            return Ok(Err("HTTP requires approved execution".into()));
        }
        Ok(self
            .policy
            .http_get_cancelled(&self.workspace, &url, self.cancel.clone())
            .map_err(|e| e.to_string())
            .and_then(bounded))
    }
    fn kv_get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        self.require("kv")?;
        Ok(self.kv.get(&key).cloned())
    }
    fn kv_set(&mut self, key: String, value: String) -> wasmtime::Result<Result<(), String>> {
        self.require("kv")?;
        if key.len() > 256
            || value.len() > 65536
            || (!self.kv.contains_key(&key) && self.kv.len() >= 256)
        {
            return Ok(Err("KV limit".into()));
        }
        self.kv.insert(key, value);
        Ok(Ok(()))
    }
}
fn bounded(v: String) -> Result<String, String> {
    if v.len() > 1 << 20 {
        Err("host response limit".into())
    } else {
        Ok(v)
    }
}
impl EngineHost {
    pub fn new() -> AppResult<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true);
        let engine = Engine::new(&config).map_err(fail)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(fail)?;
        bindings::Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s).map_err(fail)?;
        let weak = engine.weak();
        std::thread::Builder::new()
            .name("harness-plugin-epoch".into())
            .spawn(move || {
                while let Some(e) = weak.upgrade() {
                    e.increment_epoch();
                    drop(e);
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(fail)?;
        Ok(Self { engine, linker })
    }
    pub fn compile(&self, path: &Path) -> AppResult<Component> {
        Component::from_file(&self.engine, path).map_err(fail)
    }
    pub fn execute(
        &self,
        request: ExecutionRequest<'_>,
    ) -> AppResult<(String, BTreeMap<String, String>)> {
        let ExecutionRequest {
            component,
            workspace,
            root,
            grants,
            policy,
            kv,
            name,
            args,
            cancel,
            disabled,
            approved_execution,
        } = request;
        let native_cancel = CancellationToken::new();
        let native_signal = native_cancel.clone();
        let external_cancel = cancel.clone();
        let plugin_disabled = disabled.clone();
        let native_watch = tokio::runtime::Handle::current().spawn(async move {
            tokio::select! { _=external_cancel.cancelled()=>{}, _=plugin_disabled.cancelled()=>{} };
            native_signal.cancel();
        });
        struct Watch(tokio::task::JoinHandle<()>);
        impl Drop for Watch {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _watch = Watch(native_watch);
        let data = Data {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(256 << 20)
                .memories(1)
                .instances(32)
                .tables(8)
                .build(),
            kv,
            grants,
            workspace: workspace.into(),
            root: root.into(),
            policy,
            approved: false,
            cancel: native_cancel,
            external_cancel: cancel.clone(),
            disabled: disabled.clone(),
        };
        let mut store = Store::new(&self.engine, data);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(5_000_000_000).map_err(fail)?;
        store.set_epoch_deadline(1);
        let start = std::time::Instant::now();
        let deadline = Arc::new(std::sync::atomic::AtomicU64::new(5000));
        let deadline_check = deadline.clone();
        store.epoch_deadline_callback(move |_| {
            if cancel.is_cancelled()
                || disabled.is_cancelled()
                || start.elapsed().as_millis()
                    > deadline_check.load(std::sync::atomic::Ordering::Relaxed) as u128
            {
                Err(wasmtime::Error::msg("plugin cancelled/deadline"))
            } else {
                Ok(wasmtime::UpdateDeadline::Continue(1))
            }
        });
        // Instantiation and lifecycle each share a strict 5 second budget. Tool receives a fresh 60s budget.
        store.set_epoch_deadline(1);
        let guest =
            bindings::Plugin::instantiate(&mut store, component, &self.linker).map_err(fail)?;
        let started = std::time::Instant::now();
        guest
            .call_activate(&mut store)
            .map_err(fail)?
            .map_err(fail)?;
        if started.elapsed() > Duration::from_secs(5) {
            return Err(fail("activation deadline"));
        }
        store.set_fuel(5_000_000_000).map_err(fail)?;
        deadline.store(
            start.elapsed().as_millis() as u64
                + if name.starts_with("hook:") {
                    5000
                } else {
                    60_000
                },
            std::sync::atomic::Ordering::Relaxed,
        );
        store.data_mut().approved = approved_execution;
        let result = guest
            .call_execute(&mut store, name, args)
            .map_err(fail)?
            .map_err(fail)?;
        store.data_mut().approved = false;
        bounded(result.clone()).map_err(fail)?;
        deadline.store(
            start.elapsed().as_millis() as u64 + 5000,
            std::sync::atomic::Ordering::Relaxed,
        );
        let started = std::time::Instant::now();
        guest
            .call_deactivate(&mut store)
            .map_err(fail)?
            .map_err(fail)?;
        if started.elapsed() > Duration::from_secs(5) {
            return Err(fail("deactivation deadline"));
        }
        Ok((result, store.into_data().kv))
    }
}
