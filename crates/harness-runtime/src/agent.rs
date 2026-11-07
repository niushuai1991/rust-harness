//! Scoped agent ownership, durable tool settlements, and human approvals.
use crate::model::{MessageRole, ModelMessage, ModelProvider, StreamDelta, cancelled};
use harness_contracts::*;
use harness_extensions::{HostPolicy, PluginManager};
use harness_platform::{ShellService, WorkspaceManager};
use harness_storage::Store;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

type PendingApproval = (ApprovalRequest, oneshot::Sender<bool>);
static FILE_MUTATION: Mutex<()> = Mutex::new(());
#[derive(Default)]
pub struct Approvals {
    pending: Mutex<HashMap<String, PendingApproval>>,
}
impl Approvals {
    pub fn list(&self) -> Vec<ApprovalRequest> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .map(|(r, _)| r.clone())
            .collect()
    }
    pub fn resolve(&self, id: &str, allow: bool) -> AppResult<()> {
        let (_, tx) = self
            .pending
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| AppError::new("APPROVAL_EXPIRED", "审批不存在或已过期"))?;
        tx.send(allow)
            .map_err(|_| AppError::new("APPROVAL_EXPIRED", "调用已结束"))
    }
}

pub struct Runtime {
    pub store: Store,
    pub models: crate::providers::ModelService,
    pub presets: crate::presets::PresetService,
    pub skills: crate::skills::SkillService,
    pub workspaces: WorkspaceManager,
    pub plugins: Arc<PluginManager>,
    pub approvals: Approvals,
    pub mcp: harness_extensions::mcp::McpManager,
    pub lsp: Arc<harness_extensions::lsp::LspManager>,
    mcp_scopes: Mutex<HashMap<String, String>>,
    events: broadcast::Sender<LiveEvent>,
    pub(crate) active: Mutex<HashMap<String, CancellationToken>>,
    queues: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    cancelled_messages: Mutex<HashSet<(String, String)>>,
    children: Mutex<HashMap<String, Vec<String>>>,
    parents: Mutex<HashMap<String, String>>,
    custom: crate::registry::Tools,
    _host_lock: std::fs::File,
}
struct ToolExecution<'a> {
    session: &'a str,
    workspace: &'a str,
    root: &'a Path,
    call: &'a ToolCall,
    reads: &'a mut HashMap<String, String>,
    provider: Arc<dyn ModelProvider>,
    cancel: CancellationToken,
}
impl Runtime {
    pub async fn open(data_dir: PathBuf) -> AppResult<Arc<Self>> {
        harness_extensions::mcp::init_tls();
        std::fs::create_dir_all(&data_dir).map_err(internal)?;
        let host_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data_dir.join("host.lock"))
            .map_err(internal)?;
        host_lock.try_lock().map_err(|_| {
            AppError::new(
                "HOST_RUNNING",
                "此数据目录已由另一个宿主使用；请关闭现有实例或指定独立数据目录",
            )
        })?;
        let store = Store::open(&data_dir.join("harness.db")).await?;
        store.recover().await?;
        let workspaces = WorkspaceManager::new(data_dir.clone())?;
        let plugins = Arc::new(PluginManager::new(data_dir.join("plugins"))?);
        plugins.set_host_policy(Arc::new(ReplicaHost));
        let (events, _) = broadcast::channel(4096);
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut parents: HashMap<String, String> = HashMap::new();
        for session in store.sessions(None).await? {
            for event in store.events(&session.id, 0).await? {
                if event.kind == "subagent.created"
                    && event.data["historyOnly"] != true
                    && let Some(child) = event.data["sessionId"].as_str()
                    && let Ok(child_info) = store.session(child).await
                    && child_info.workspace_id == session.workspace_id
                    && !children
                        .values()
                        .any(|ids| ids.iter().any(|id| id == child))
                {
                    children
                        .entry(session.id.clone())
                        .or_default()
                        .push(child.into());
                    parents.insert(child.into(), session.id.clone());
                }
            }
        }
        let models = crate::providers::ModelService::new(store.clone());
        Ok(Arc::new(Self {
            presets: crate::presets::PresetService::new(store.clone(), models.clone()),
            skills: crate::skills::SkillService::new(store.clone()),
            models,
            store,
            workspaces,
            plugins,
            approvals: Approvals::default(),
            mcp: harness_extensions::mcp::McpManager::new(),
            lsp: Arc::new(harness_extensions::lsp::LspManager::new()),
            mcp_scopes: Mutex::new(HashMap::new()),
            events,
            active: Mutex::new(HashMap::new()),
            queues: Mutex::new(HashMap::new()),
            cancelled_messages: Mutex::new(HashSet::new()),
            children: Mutex::new(children),
            parents: Mutex::new(parents),
            custom: Arc::new(Mutex::new(Default::default())),
            _host_lock: host_lock,
        }))
    }
    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.events.subscribe()
    }
    pub fn active_count(&self) -> usize {
        self.active.lock().unwrap().len()
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.active.lock().unwrap().contains_key(id)
    }
    pub fn children_of(&self, session: &str) -> Vec<String> {
        self.children
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default()
    }
    /// 子代理归属的父会话；非子代理返回 None。
    pub fn parent_of(&self, session: &str) -> Option<String> {
        self.parents.lock().unwrap().get(session).cloned()
    }
    /// 会话列表并附加父子关系；`workspace` 过滤工作区。
    pub async fn list_sessions(&self, workspace: Option<&str>) -> AppResult<Vec<SessionInfo>> {
        let mut sessions = self.store.sessions(workspace).await?;
        let parents = self.parents.lock().unwrap();
        for session in &mut sessions {
            if let Some(parent) = parents.get(&session.id) {
                session.parent_id = Some(parent.clone());
            }
        }
        Ok(sessions)
    }
    /// 正在执行回合的会话标识，供界面展示子代理等后台活动状态。
    pub fn active_ids(&self) -> Vec<String> {
        self.active.lock().unwrap().keys().cloned().collect()
    }
    pub fn adopt_child(&self, parent: &str, child: &str) -> AppResult<()> {
        if parent == child {
            return Err(AppError::new("AGENT_SCOPE", "不能将自己登记为子代理"));
        }
        let mut children = self.children.lock().unwrap();
        if children
            .values()
            .any(|ids| ids.iter().any(|id| id == child))
        {
            return Err(AppError::new("AGENT_SCOPE", "子代理已归其他所有者管理"));
        }
        children
            .entry(parent.into())
            .or_default()
            .push(child.into());
        self.parents
            .lock()
            .unwrap()
            .insert(child.into(), parent.into());
        Ok(())
    }
    pub async fn session_boundary(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let queue = self
            .queues
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        queue.lock_owned().await
    }
    pub fn register_tool(
        &self,
        tool: Arc<dyn crate::ExternalTool>,
    ) -> AppResult<crate::Registration> {
        let name = tool.definition().name;
        let mut registry = self.custom.lock().unwrap();
        if name.is_empty()
            || builtin_tools().iter().any(|t| t.name == name)
            || name.starts_with("plugin__")
            || name.starts_with("mcp__")
            || registry.contains_key(&name)
        {
            return Err(AppError::new("DUPLICATE_TOOL", name));
        }
        registry.insert(name.clone(), tool);
        Ok(crate::Registration {
            name,
            registry: Arc::downgrade(&self.custom),
        })
    }
    pub fn cancel(&self, id: &str) {
        if let Some(token) = self.active.lock().unwrap().get(id) {
            token.cancel();
        }
    }
    pub fn cancel_all(&self) {
        for token in self.active.lock().unwrap().values() {
            token.cancel();
        }
    }
    /// This entrypoint is exposed only to explicit human configuration commands, never model tools.
    pub async fn connect_mcp(
        &self,
        workspace: &str,
        config: harness_extensions::mcp::ServerConfig,
    ) -> AppResult<()> {
        use harness_extensions::mcp::ServerConfig;
        let info = self.workspaces.get(workspace)?;
        if self.mcp_scopes.lock().unwrap().contains_key(config.id()) {
            return Err(AppError::new("MCP_DUPLICATE", "请先断开同名MCP连接"));
        }
        match &config {
            ServerConfig::StreamableHttp { .. } => self.mcp.connect_http(&config, true).await?,
            ServerConfig::Stdio { command, args, .. } => {
                let child = ShellService::spawn_stdio(
                    Path::new(&info.replica_path),
                    command,
                    args,
                    CancellationToken::new(),
                )
                .await?;
                let (read, write) = tokio::io::split(child);
                self.mcp
                    .connect_sandboxed_stdio(&config, read, write, true)
                    .await?;
            }
        }
        self.mcp_scopes
            .lock()
            .unwrap()
            .insert(config.id().into(), workspace.into());
        Ok(())
    }
    pub async fn disconnect_mcp(&self, id: &str) -> AppResult<()> {
        self.mcp.disconnect(id).await?;
        self.mcp_scopes.lock().unwrap().remove(id);
        Ok(())
    }
    /// Test and SDK seam: register an LSP connection whose pipes the caller created.
    /// Production spawns the server via `ShellService::spawn_stdio` in `ensure_running`.
    pub async fn connect_lsp_streams<R, W>(
        &self,
        workspace: &str,
        config: &LspServerConfig,
        read: R,
        write: W,
    ) -> AppResult<Arc<harness_extensions::lsp::LspConnection>>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let info = self.workspaces.get(workspace)?;
        let replica = PathBuf::from(&info.replica_path);
        self.lsp
            .connect(workspace, config, &replica, read, write)
            .await
    }
    pub async fn lsp_status(&self) -> Vec<LspStatusEntry> {
        self.lsp.status().await
    }
    /// Drop the server connection; the next LSP tool call respawns it from config.
    pub async fn lsp_restart(&self, workspace: &str, server_id: &str) -> AppResult<()> {
        self.lsp.disconnect(workspace, server_id).await
    }
    async fn lsp_servers(&self) -> AppResult<Vec<LspServerConfig>> {
        match self.store.get("lsp.servers.v1").await? {
            Some(value) => serde_json::from_value(value)
                .map_err(|e| AppError::new("LSP_CONFIG", format!("lsp.servers.v1 配置无效：{e}"))),
            None => Ok(LspServerConfig::default_servers()),
        }
    }
    async fn ensure_running(
        &self,
        workspace: &str,
        server: &LspServerConfig,
    ) -> AppResult<Arc<harness_extensions::lsp::LspConnection>> {
        if let Some(existing) = self.lsp.connection(workspace, &server.id).await {
            return Ok(existing);
        }
        let info = self.workspaces.get(workspace)?;
        let replica = PathBuf::from(&info.replica_path);
        let child = ShellService::spawn_stdio(
            &replica,
            &server.command,
            &server.args,
            CancellationToken::new(),
        )
        .await
        .map_err(|e| {
            AppError::new(
                e.code.clone(),
                format!("{e}；可通过 settings 键 lsp.servers.v1 配置语言服务器"),
            )
        })?;
        let (read, write) = tokio::io::split(child);
        self.lsp
            .connect(workspace, server, &replica, read, write)
            .await
            .map_err(|e| {
                AppError::new(
                    e.code.clone(),
                    format!("{e}；可通过 settings 键 lsp.servers.v1 调整配置"),
                )
            })
    }
    /// Resolve the configured server for a file extension, ensure it runs, and sync the file.
    async fn ensure_lsp(
        &self,
        workspace: &str,
        root: &Path,
        servers: &[LspServerConfig],
        path: &str,
    ) -> AppResult<(Arc<harness_extensions::lsp::LspConnection>, String)> {
        let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let server = servers
            .iter()
            .find(|s| {
                s.extensions
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&extension))
            })
            .ok_or_else(|| {
                AppError::new(
                    "LSP_NO_SERVER",
                    format!(
                        "未配置处理 .{extension} 的语言服务器；可在 settings 键 lsp.servers.v1 配置"
                    ),
                )
            })?;
        let connection = self.ensure_running(workspace, server).await?;
        let text = harness_platform::read_file(root, path)?;
        let uri = harness_extensions::lsp::file_uri(root, path)?;
        connection
            .sync_document(
                &uri,
                harness_extensions::lsp::language_for_extension(&extension),
                &text,
            )
            .await?;
        Ok((connection, uri))
    }
    async fn execute_lsp_tool(
        &self,
        name: &str,
        workspace: &str,
        root: &Path,
        a: &Value,
        cancel: CancellationToken,
    ) -> AppResult<String> {
        let servers = self.lsp_servers().await?;
        match name {
            "lsp_diagnostics" => {
                let Some(path) = a["path"].as_str() else {
                    let mut all = Vec::new();
                    for server in &servers {
                        if let Some(connection) = self.lsp.connection(workspace, &server.id).await {
                            let value = connection.diagnostics(None).await;
                            if let Some(list) = value.as_array() {
                                all.extend(list.iter().cloned());
                            }
                        }
                    }
                    return serde_json::to_string(&Value::Array(all)).map_err(internal);
                };
                let (connection, uri) = self.ensure_lsp(workspace, root, &servers, path).await?;
                if a["wait"].as_bool().unwrap_or(false) {
                    connection
                        .wait_diagnostics(&uri, LSP_DIAGNOSTICS_IDLE, LSP_DIAGNOSTICS_CAP, &cancel)
                        .await?;
                }
                serde_json::to_string(&connection.diagnostics(Some(&uri)).await).map_err(internal)
            }
            "lsp_hover" | "lsp_definition" | "lsp_references" => {
                let path = arg(a, "path")?;
                let (connection, uri) = self.ensure_lsp(workspace, root, &servers, path).await?;
                let mut params = json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": lsp_position(a, "line")?, "character": lsp_position(a, "character")?}
                });
                let method = match name {
                    "lsp_hover" => "textDocument/hover",
                    "lsp_definition" => "textDocument/definition",
                    _ => {
                        params["context"] = json!({"includeDeclaration": a["includeDeclaration"].as_bool().unwrap_or(true)});
                        "textDocument/references"
                    }
                };
                lsp_text(connection.request(method, params, &cancel).await?)
            }
            "lsp_symbols" => {
                if let Some(path) = a["path"].as_str() {
                    let (connection, uri) =
                        self.ensure_lsp(workspace, root, &servers, path).await?;
                    lsp_text(
                        connection
                            .request(
                                "textDocument/documentSymbol",
                                json!({"textDocument": {"uri": uri}}),
                                &cancel,
                            )
                            .await?,
                    )
                } else {
                    let query = arg(a, "query")?;
                    let server = servers
                        .first()
                        .ok_or_else(|| AppError::new("LSP_NO_SERVER", "未配置任何语言服务器"))?;
                    let connection = self.ensure_running(workspace, server).await?;
                    lsp_text(
                        connection
                            .request("workspace/symbol", json!({"query": query}), &cancel)
                            .await?,
                    )
                }
            }
            _ => Err(AppError::new("TOOL_UNAVAILABLE", name)),
        }
    }
    pub fn mcp_connections(&self) -> Vec<Value> {
        self.mcp_scopes
            .lock()
            .unwrap()
            .iter()
            .map(|(id, workspace)| json!({"id":id,"workspaceId":workspace}))
            .collect()
    }
    pub async fn append(&self, id: &str, kind: &str, data: Value) -> AppResult<SessionEvent> {
        let event = self.store.append(id, kind, data).await?;
        let _ = self.events.send(LiveEvent::Durable {
            event: event.clone(),
        });
        Ok(event)
    }
    pub fn run_session(
        self: Arc<Self>,
        session_id: String,
        text: String,
        provider: Arc<dyn ModelProvider>,
    ) -> futures::future::BoxFuture<'static, AppResult<()>> {
        self.run_session_with_attachments(session_id, text, provider, vec![])
    }
    pub fn run_session_with_attachments(
        self: Arc<Self>,
        session_id: String,
        text: String,
        provider: Arc<dyn ModelProvider>,
        attachments: Vec<String>,
    ) -> futures::future::BoxFuture<'static, AppResult<()>> {
        Box::pin(async move {
            let message_id = self.enqueue_message(&session_id, text, attachments).await?;
            self.execute_message(session_id, message_id, provider).await
        })
    }
    /// 校验输入并落 `inbox.queued`，立即返回消息标识；回合执行经 [`Self::execute_message`]。
    pub async fn enqueue_message(
        self: &Arc<Self>,
        session_id: &str,
        text: String,
        attachments: Vec<String>,
    ) -> AppResult<String> {
        if (text.trim().is_empty() && attachments.is_empty())
            || text.len() > 1024 * 1024
            || attachments.len() > 20
        {
            return Err(AppError::new("INVALID_INPUT", "消息不能为空或超过1MiB"));
        }
        self.store.session(session_id).await?;
        let mut attached = Vec::new();
        for hash in attachments {
            let metadata = self.artifact_metadata(session_id, &hash).await?;
            self.store.blob(&hash).await?;
            if !attached.iter().any(|a: &ArtifactMetadata| a.hash == hash) {
                attached.push(metadata);
            }
        }
        let message_id = new_id();
        self.append(
            session_id,
            "inbox.queued",
            json!({"id":message_id,"content":text,"attachments":attached}),
        )
        .await?;
        Ok(message_id)
    }
    /// 标记排队中的消息取消；`execute_message` 获得队列锁后结算为 `inbox.cancelled`。
    pub fn cancel_message(&self, session: &str, message_id: &str) {
        self.cancelled_messages
            .lock()
            .unwrap()
            .insert((session.to_owned(), message_id.to_owned()));
    }
    /// 执行已入队的消息；回合事件携带 `messageId`，排队时取消的消息不产生回合事件、不调用模型。
    pub async fn execute_message(
        self: Arc<Self>,
        session_id: String,
        message_id: String,
        provider: Arc<dyn ModelProvider>,
    ) -> AppResult<()> {
        let session = self.store.session(&session_id).await?;
        let queued = self
            .store
            .events(&session_id, 0)
            .await?
            .into_iter()
            .find(|e| {
                e.kind == "inbox.queued" && e.data["id"].as_str() == Some(message_id.as_str())
            })
            .ok_or_else(|| AppError::new("INVALID_MESSAGE", "消息不存在或已结算"))?;
        let queue = self
            .queues
            .lock()
            .unwrap()
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _serial = queue.lock().await;
        if self
            .cancelled_messages
            .lock()
            .unwrap()
            .remove(&(session_id.clone(), message_id.clone()))
        {
            self.append(&session_id, "inbox.cancelled", json!({"id":message_id}))
                .await?;
            return Err(AppError::new("CANCELLED", "消息已在排队时取消"));
        }
        let lease = self.workspaces.acquire(&session.workspace_id)?;
        let cancel = CancellationToken::new();
        self.active
            .lock()
            .unwrap()
            .insert(session_id.clone(), cancel.clone());
        let run_id = new_id();
        let result = async {
            self.append(
                &session_id,
                "turn.start",
                json!({"runId":run_id,"messageId":message_id}),
            )
            .await?;
            self.append(&session_id, "inbox.claimed", json!({"id":message_id}))
                .await?;
            let text = queued.data["content"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let attached: Vec<ArtifactMetadata> =
                serde_json::from_value(queued.data["attachments"].clone()).map_err(internal)?;
            self.append(
                &session_id,
                "user.message",
                json!({"content":text,"attachments":attached}),
            )
            .await?;
            self.load_skill_mentions(&session_id, &text, Path::new(&lease.info.replica_path))
                .await?;
            self.steps(
                &session_id,
                &session.workspace_id,
                Path::new(&lease.info.replica_path),
                provider,
                cancel.clone(),
            )
            .await
        }
        .await;
        self.active.lock().unwrap().remove(&session_id);
        let status = match &result {
            Ok(()) => "completed",
            Err(e) if e.code == "CANCELLED" => "cancelled",
            Err(_) => "error",
        };
        let ending = self
            .append(
                &session_id,
                "turn.end",
                json!({"runId":run_id,"messageId":message_id,"status":status,"error":result.as_ref().err()}),
            )
            .await;
        let _ = self.events.send(LiveEvent::Status {
            session_id: session_id.clone(),
            status: status.into(),
        });
        ending?;
        result
    }
    /// 解析用户消息中的 `$name` 提及：命中启用技能时 append `skill.loaded` 注入正文。
    /// 单个技能加载失败仅告警跳过，不阻塞回合。
    async fn load_skill_mentions(&self, session: &str, text: &str, root: &Path) -> AppResult<()> {
        let skills = self.skills.list(root).await?;
        let known: std::collections::BTreeSet<String> = skills
            .iter()
            .filter(|s| s.enabled && s.broken_reason.is_none())
            .map(|s| s.name.clone())
            .collect();
        for name in crate::skills::extract_mentions(text, &known) {
            match self.skills.resolve(root, &name).await {
                Ok(Some(resolved)) => {
                    self.append(
                        session,
                        "skill.loaded",
                        json!({
                            "name":resolved.info.name,
                            "path":resolved.info.path,
                            "content":resolved.content
                        }),
                    )
                    .await?;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(session, name, error = %e, "技能加载失败，跳过"),
            }
        }
        Ok(())
    }
    async fn steps(
        self: &Arc<Self>,
        session: &str,
        workspace: &str,
        root: &Path,
        provider: Arc<dyn ModelProvider>,
        cancel: CancellationToken,
    ) -> AppResult<()> {
        self.plugins.set_workspace_policy(
            workspace,
            vec![
                "tools".into(),
                "hooks".into(),
                "log".into(),
                "workspace.read".into(),
                "workspace.write".into(),
                "http".into(),
                "kv".into(),
            ],
        );
        let skills_catalog = self.skills.catalog(root).await?;
        let context = workspace_context(root, &skills_catalog)?;
        self.append(session, "context.snapshot", json!({"content":context}))
            .await?;
        let mut reads = HashMap::new();
        for step in 0..30 {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let mut tools = builtin_tools();
            tools.extend(self.plugins.tool_definitions(workspace)?);
            tools.extend(self.custom.lock().unwrap().values().map(|t| t.definition()));
            let scoped_servers = self
                .mcp_scopes
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, w)| w.as_str() == workspace)
                .map(|(id, _)| format!("mcp__{id}__"))
                .collect::<Vec<_>>();
            tools.extend(
                self.mcp
                    .tool_definitions()
                    .await
                    .into_iter()
                    .filter(|t| scoped_servers.iter().any(|p| t.name.starts_with(p))),
            );
            let context_hooks = self
                .plugins
                .run_hook(
                    workspace,
                    root,
                    "before_request",
                    json!({"sessionId":session,"step":step}),
                    cancel.clone(),
                )
                .await?;
            for context in context_hooks {
                self.append(
                    session,
                    "plugin.context",
                    json!({"content":context["context"]}),
                )
                .await?;
            }
            let history = self.store.events(session, 0).await?;
            let messages = project_messages(&history);
            let request_id = new_id();
            self.append(
                session,
                "request.snapshot",
                json!({"id":request_id,"step":step,"messages":messages,"tools":tools,"route":provider.route(),"request":provider.request_snapshot(&messages,&tools)?}),
            )
            .await?;
            let sender = self.events.clone();
            let sid = session.to_owned();
            let rid = request_id.clone();
            let index = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let partial = Arc::new(Mutex::new((String::new(), String::new())));
            let partial_sink = partial.clone();
            let sink = Arc::new(move |d: StreamDelta| {
                let mut p = partial_sink.lock().unwrap();
                if d.reasoning {
                    p.1.push_str(&d.text);
                } else {
                    p.0.push_str(&d.text);
                }
                drop(p);
                let _ = sender.send(LiveEvent::Delta {
                    session_id: sid.clone(),
                    request_id: rid.clone(),
                    index: index.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    text: d.text,
                    reasoning: d.reasoning,
                });
            });
            let answer = match provider
                .complete(messages, tools.clone(), cancel.clone(), sink)
                .await
            {
                Ok(answer) => answer,
                Err(e) => {
                    let p = partial.lock().unwrap().clone();
                    self.append(
                        session,
                        "assistant.attempt",
                        json!({"requestId":request_id,"content":p.0,"reasoning":p.1,"error":e}),
                    )
                    .await?;
                    return Err(e);
                }
            };
            let mut ids: BTreeSet<String> = history
                .iter()
                .filter(|e| e.kind == "tool.call")
                .filter_map(|e| e.data["id"].as_str().map(str::to_owned))
                .collect();
            if answer
                .calls
                .iter()
                .any(|c| c.id.is_empty() || c.name.is_empty() || !ids.insert(c.id.clone()))
            {
                let error = AppError::new("MODEL_TOOL", "工具批次包含空或重复的调用标识");
                self.append(
                    session,
                    "assistant.attempt",
                    json!({"content":answer.content,"reasoning":answer.reasoning,"error":error}),
                )
                .await?;
                return Err(error);
            }
            let mut records = vec![(
                "assistant.message".into(),
                json!({"content":answer.content,"reasoning":answer.reasoning,"toolCalls":answer.calls,"usage":answer.usage,"replay":answer.replay}),
            )];
            for call in &answer.calls {
                records.push((
                    "tool.call".into(),
                    serde_json::to_value(call).map_err(internal)?,
                ));
            }
            for event in self.store.append_batch(session, records).await? {
                let _ = self.events.send(LiveEvent::Durable { event });
            }
            if answer.calls.is_empty() {
                return Ok(());
            }
            for mut call in answer.calls {
                let result=async{
                    let definition=tools.iter().find(|t|t.name==call.name).ok_or_else(||AppError::new("TOOL_UNAVAILABLE",&call.name))?;
                    let schema=jsonschema::validator_for(&definition.parameters).map_err(internal)?;
                    if !schema.is_valid(&call.arguments){return Err(AppError::new("TOOL_ARGUMENTS","工具参数不符合JSON Schema"));}
                    if cancel.is_cancelled(){return Err(cancelled());}
                    let original=serde_json::to_value(&call).map_err(internal)?;
                    for rewrite in self.plugins.run_hook(workspace,root,"before_tool",original.clone(),cancel.clone()).await?{
                        call.name=rewrite["name"].as_str().ok_or_else(||AppError::new("PLUGIN_HOOK","缺少工具名称"))?.into();call.arguments=rewrite["arguments"].clone();
                    }
                    if original!=serde_json::to_value(&call).map_err(internal)?{self.append(session,"tool.rewrite",json!({"callId":call.id,"original":original,"effective":call})).await?;}
                    let effective=tools.iter().find(|t|t.name==call.name).ok_or_else(||AppError::new("TOOL_UNAVAILABLE",&call.name))?;
                    if !jsonschema::validator_for(&effective.parameters).map_err(internal)?.is_valid(&call.arguments){return Err(AppError::new("TOOL_ARGUMENTS","钩子修改后的工具参数无效"));}
                    let needs_approval=self.custom.lock().unwrap().get(&call.name).map(|t|t.requires_approval()).unwrap_or_else(||requires_approval(&call.name));
                    if needs_approval{self.approve(session,&call,&cancel).await?;}
                    let tool_cancel=cancel.child_token();
                    let work = self.execute_tool(ToolExecution {
                        session,
                        workspace,
                        root,
                        call: &call,
                        reads: &mut reads,
                        provider: provider.clone(),
                        cancel: tool_cancel.clone(),
                    });
                    tokio::pin!(work);
                    tokio::select!{
                        _ = cancel.cancelled()=>{tool_cancel.cancel();let _=work.await;Err(cancelled())},
                        _ = tokio::time::sleep(std::time::Duration::from_secs(180))=>{tool_cancel.cancel();let _=work.await;Err(AppError::new("TOOL_TIMEOUT","工具超过180秒执行时限"))},
                        r=&mut work=>r,
                    }
                }.await;
                let status = match &result {
                    Ok(_) => "completed",
                    Err(e) if matches!(e.code.as_str(), "CANCELLED" | "TOOL_TIMEOUT") => "unknown",
                    Err(e) if e.code == "APPROVAL_DENIED" => "denied",
                    Err(_) => "error",
                };
                let protected_failure = result.is_err();
                let mut result = match result {
                    Ok(r) => r,
                    Err(e) => ToolResult {
                        content: if status == "unknown" {
                            format!("{e}；操作可能已产生副作用，结果未知，未自动重试。")
                        } else {
                            e.to_string()
                        },
                        is_error: true,
                    },
                };
                if !cancel.is_cancelled() {
                    match self
                        .plugins
                        .run_hook(
                            workspace,
                            root,
                            "after_tool",
                            json!({"call":call,"result":result}),
                            cancel.clone(),
                        )
                        .await
                    {
                        Ok(rewrites) => {
                            for rewrite in rewrites {
                                result = serde_json::from_value(rewrite).map_err(internal)?;
                            }
                        }
                        Err(error) => {
                            result.content.push_str(&format!("\n后置钩子失败：{error}"));
                            result.is_error = true;
                        }
                    }
                }
                if protected_failure {
                    result.is_error = true;
                }
                let mut content = result.content;
                let mut artifact = None;
                if content.len() > 64 * 1024 {
                    let hash = self.store.put_blob(content.as_bytes()).await?;
                    artifact = Some(ArtifactMetadata {
                        hash: hash.clone(),
                        name: format!("{}-output.txt", call.name),
                        media_type: "text/plain; charset=utf-8".into(),
                        size: content.len() as u32,
                    });
                    let preview = content.chars().take(12000).collect::<String>();
                    content = format!(
                        "{preview}\n\n[完整输出 artifact:{hash}；使用 read_artifact 分块读取]"
                    );
                }
                self.append(session,"tool.result",json!({"callId":call.id,"name":call.name,"content":content,"isError":result.is_error,"status":status,"artifact":artifact})).await?;
            }
        }
        Err(AppError::new("STEP_LIMIT", "回合达到30次模型调用上限"))
    }
    pub(crate) async fn approve(
        &self,
        session: &str,
        call: &ToolCall,
        cancel: &CancellationToken,
    ) -> AppResult<()> {
        let request = ApprovalRequest {
            id: new_id(),
            session_id: session.into(),
            tool: call.name.clone(),
            arguments: call.arguments.clone(),
            reason: "允许此工具在隔离工作副本中执行？原项目仍需单独确认同步。".into(),
        };
        let (tx, rx) = oneshot::channel();
        self.approvals
            .pending
            .lock()
            .unwrap()
            .insert(request.id.clone(), (request.clone(), tx));
        let logged = self
            .append(
                session,
                "approval.request",
                serde_json::to_value(&request).map_err(internal)?,
            )
            .await;
        if let Err(e) = logged {
            self.approvals.pending.lock().unwrap().remove(&request.id);
            return Err(e);
        }
        let allowed = tokio::select! {_ = cancel.cancelled()=>false,r=rx=>r.unwrap_or(false),_ = tokio::time::sleep(std::time::Duration::from_secs(600))=>false};
        self.approvals.pending.lock().unwrap().remove(&request.id);
        self.append(
            session,
            "approval.resolved",
            json!({"id":request.id,"allow":allowed}),
        )
        .await?;
        if allowed {
            Ok(())
        } else {
            Err(AppError::new(
                "APPROVAL_DENIED",
                "用户拒绝、取消或审批已过期",
            ))
        }
    }
    async fn execute_tool(self: &Arc<Self>, input: ToolExecution<'_>) -> AppResult<ToolResult> {
        let ToolExecution {
            session,
            workspace,
            root,
            call,
            reads,
            provider,
            cancel,
        } = input;
        let a = &call.arguments;
        let text = match call.name.as_str() {
            "read_artifact" => {
                let offset = a["offset"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", "无效偏移"))?;
                let length = a["length"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", "无效长度"))?;
                serde_json::to_string(
                    &self
                        .artifact_read(session, arg(a, "hash")?, offset, length)
                        .await?,
                )
                .map_err(internal)?
            }
            "read" => {
                let path = arg(a, "path")?;
                let value = harness_platform::read_file(root, path)?;
                reads.insert(path.into(), value.clone());
                value
            }
            "write" => {
                let _mutation = FILE_MUTATION.lock().unwrap();
                let path = arg(a, "path")?;
                let existing = harness_platform::resolve_path(root, path, false)?;
                if existing.exists() {
                    let current = harness_platform::read_file(root, path)?;
                    if reads.get(path) != Some(&current) {
                        return Err(AppError::new(
                            "READ_BEFORE_EDIT",
                            "修改已有文件前必须读取当前版本",
                        ));
                    }
                }
                let content = arg(a, "content")?;
                harness_platform::write_file(root, path, content)?;
                reads.insert(path.into(), content.into());
                format!("已写入副本 {path}")
            }
            "edit" => {
                let _mutation = FILE_MUTATION.lock().unwrap();
                let path = arg(a, "path")?;
                if !reads.contains_key(path) {
                    return Err(AppError::new("READ_BEFORE_EDIT", "edit前必须read"));
                }
                let old = arg(a, "oldText")?;
                let new = arg(a, "newText")?;
                let content = harness_platform::read_file(root, path)?;
                if reads.get(path) != Some(&content) {
                    return Err(AppError::new(
                        "EDIT_CONFLICT",
                        "读取后文件已改变，请重新read",
                    ));
                }
                if old.is_empty() || content.matches(old).count() != 1 {
                    return Err(AppError::new(
                        "EDIT_CONFLICT",
                        "待替换文本必须在当前文件中恰好出现一次",
                    ));
                }
                let updated = content.replacen(old, new, 1);
                harness_platform::write_file(root, path, &updated)?;
                reads.insert(path.into(), updated);
                format!("已修改副本 {path}")
            }
            "list_files" => {
                serde_json::to_string(&harness_platform::list_files(root)?).map_err(internal)?
            }
            "search" => {
                let query = arg(a, "query")?;
                let mut found = Vec::new();
                for path in harness_platform::list_files(root)?.into_iter().take(10000) {
                    if let Ok(text) = harness_platform::read_file(root, &path) {
                        for (i, line) in text.lines().enumerate() {
                            if line.contains(query) {
                                found.push(format!(
                                    "{path}:{}:{}",
                                    i + 1,
                                    line.chars().take(300).collect::<String>()
                                ));
                                if found.len() >= 200 {
                                    break;
                                }
                            }
                        }
                    }
                    if found.len() >= 200 {
                        break;
                    }
                }
                found.join("\n")
            }
            "shell" => {
                let output = ShellService::run(root, arg(a, "command")?, cancel).await?;
                return Ok(ToolResult {
                    content: serde_json::to_string(&output).map_err(internal)?,
                    is_error: output.exit_code != 0,
                });
            }
            "web_fetch" => fetch(arg(a, "url")?, cancel).await?,
            "todo_write" | "plan_write" | "goal_write" => {
                self.append(
                    session,
                    &format!("{}.state", call.name.trim_end_matches("_write")),
                    a.clone(),
                )
                .await?;
                a.to_string()
            }
            "agent_spawn" => {
                let descendants = self
                    .children
                    .lock()
                    .unwrap()
                    .values()
                    .map(Vec::len)
                    .sum::<usize>();
                if descendants >= 8 {
                    return Err(AppError::new("AGENT_LIMIT", "当前实例最多8个子代理"));
                }
                let model_id = a
                    .get("modelId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                // 先校验模型再创建子会话，失败时不留下未登记的空会话。
                let provider = match model_id {
                    Some(id) => self.models.build_model(id).await?,
                    None => provider,
                };
                let child = self
                    .store
                    .create_session(
                        workspace,
                        a.get("title").and_then(Value::as_str).unwrap_or("子任务"),
                    )
                    .await?;
                self.children
                    .lock()
                    .unwrap()
                    .entry(session.into())
                    .or_default()
                    .push(child.id.clone());
                self.parents
                    .lock()
                    .unwrap()
                    .insert(child.id.clone(), session.to_owned());
                let mut created = json!({"sessionId":child.id});
                if let Some(id) = model_id {
                    created["modelId"] = json!(id);
                }
                self.append(session, "subagent.created", created).await?;
                let runtime = self.clone();
                let id = child.id.clone();
                let task = arg(a, "task")?.to_owned();
                tokio::spawn(async move {
                    if let Err(e) = runtime.run_session(id, task, provider).await {
                        tracing::warn!(error=%e,"child agent ended");
                    }
                });
                json!({"sessionId":child.id}).to_string()
            }
            "agent_status" => {
                let ids = self
                    .children
                    .lock()
                    .unwrap()
                    .get(session)
                    .cloned()
                    .unwrap_or_default();
                let active = self.active.lock().unwrap();
                json!(
                    ids.into_iter()
                        .map(|id| json!({"active":active.contains_key(&id),"sessionId":id}))
                        .collect::<Vec<_>>()
                )
                .to_string()
            }
            "agent_interrupt" => {
                let id = arg(a, "sessionId")?;
                if !self
                    .children
                    .lock()
                    .unwrap()
                    .get(session)
                    .is_some_and(|ids| ids.iter().any(|x| x == id))
                {
                    return Err(AppError::new("AGENT_SCOPE", "只能控制本会话创建的子代理"));
                }
                self.cancel(id);
                "已请求中断".into()
            }
            name if name.starts_with("plugin__") => {
                return self
                    .plugins
                    .execute_tool(workspace, root, name, a.clone(), cancel)
                    .await;
            }
            name if name.starts_with("mcp__") => {
                return self.mcp.execute_tool(name, a.clone(), cancel).await;
            }
            name if name.starts_with("lsp_") => {
                return Ok(ToolResult {
                    content: self
                        .execute_lsp_tool(name, workspace, root, a, cancel)
                        .await?,
                    is_error: false,
                });
            }
            _ => {
                let tool = self
                    .custom
                    .lock()
                    .unwrap()
                    .get(&call.name)
                    .cloned()
                    .ok_or_else(|| AppError::new("TOOL_UNAVAILABLE", &call.name))?;
                return tool
                    .execute(
                        crate::ToolContext {
                            session_id: session.into(),
                            workspace_id: workspace.into(),
                            root: root.into(),
                            provider,
                        },
                        a.clone(),
                        cancel,
                    )
                    .await;
            }
        };
        Ok(ToolResult {
            content: text,
            is_error: false,
        })
    }
    pub async fn fork(&self, id: &str) -> AppResult<SessionInfo> {
        let _boundary = self.session_boundary(id).await;
        let original = self.store.session(id).await?;
        let events = self.store.events(id, 0).await?;
        let fork = self
            .store
            .create_session(
                &original.workspace_id,
                &format!("{} · 副本", original.title),
            )
            .await?;
        for e in events {
            let mut data = e.data;
            if e.kind == "subagent.created" {
                data["historyOnly"] = json!(true);
            }
            self.store.append(&fork.id, &e.kind, data).await?;
        }
        Ok(fork)
    }
}

fn arg<'a>(v: &'a Value, key: &str) -> AppResult<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", format!("缺少字符串字段 {key}")))
}
/// Diagnostics quiet window after the latest push or sync; hard cap for push storms.
const LSP_DIAGNOSTICS_IDLE: std::time::Duration = std::time::Duration::from_secs(3);
const LSP_DIAGNOSTICS_CAP: std::time::Duration = std::time::Duration::from_secs(120);
/// Model-facing positions are 1-based; LSP is 0-based. Zero passes through unchanged.
fn lsp_position(v: &Value, key: &str) -> AppResult<u32> {
    let value = v[key]
        .as_u64()
        .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", format!("缺少数值字段 {key}")))?;
    u32::try_from(value.saturating_sub(1))
        .map_err(|_| AppError::new("TOOL_ARGUMENTS", format!("无效 {key}")))
}
fn lsp_text(result: Value) -> AppResult<String> {
    if result.is_null() {
        Ok("无结果".into())
    } else {
        serde_json::to_string(&result).map_err(internal)
    }
}
fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::new("RUNTIME", e.to_string())
}
fn requires_approval(name: &str) -> bool {
    !matches!(
        name,
        "read"
            | "read_artifact"
            | "list_files"
            | "search"
            | "lsp_diagnostics"
            | "lsp_hover"
            | "lsp_definition"
            | "lsp_references"
            | "lsp_symbols"
            | "todo_write"
            | "plan_write"
            | "goal_write"
            | "agent_status"
    )
}
fn workspace_context(root: &Path, skills_catalog: &str) -> AppResult<String> {
    let mut context = format!(
        "你是本地开发助手。执行根为隔离工作副本 {}。文件工具只接受相对路径。先读取再编辑，原项目变更需用户在差异界面单独确认。",
        root.display()
    );
    for path in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(text) = harness_platform::read_file(root, path) {
            context.push_str(&format!(
                "\n\n{path}:\n{}",
                text.chars().take(32000).collect::<String>()
            ));
        }
    }
    if !skills_catalog.is_empty() {
        context.push_str("\n\n");
        context.push_str(skills_catalog);
    }
    Ok(context)
}
pub fn project_messages(events: &[SessionEvent]) -> Vec<ModelMessage> {
    let mut system = String::new();
    let mut history = Vec::new();
    for e in events {
        let content = || e.data["content"].as_str().unwrap_or("").to_owned();
        match e.kind.as_str() {
            "context.snapshot" => system = content(),
            "user.message" => {
                let mut message = content();
                if let Some(attachments) = e.data["attachments"].as_array() {
                    for artifact in attachments {
                        if let Ok(meta) =
                            serde_json::from_value::<ArtifactMetadata>(artifact.clone())
                        {
                            message.push_str(&format!("\n附件：{} ({}，{}字节)，hash={}。使用 read_artifact(hash,offset,length) 读取；文本为UTF-8，二进制为base64，不代表原生图像理解。",meta.name,meta.media_type,meta.size,meta.hash));
                        }
                    }
                }
                history.push(ModelMessage::user(message));
            }
            "skill.loaded" => history.push(ModelMessage::user(format!(
                "<skill name=\"{}\" path=\"{}\">\n{}\n</skill>",
                e.data["name"].as_str().unwrap_or(""),
                e.data["path"].as_str().unwrap_or(""),
                content()
            ))),
            "plugin.context" => history.push(ModelMessage::user(content())),
            "assistant.message" => history.push(ModelMessage {
                role: MessageRole::Assistant,
                content: content(),
                reasoning: e.data["reasoning"].as_str().unwrap_or("").into(),
                tool_calls: serde_json::from_value(e.data["toolCalls"].clone()).unwrap_or_default(),
                replay: serde_json::from_value(e.data["replay"].clone()).unwrap_or(None),
                ..Default::default()
            }),
            "tool.result" => history.push(ModelMessage::tool(
                e.data["callId"].as_str().unwrap_or(""),
                content(),
                e.data["isError"].as_bool().unwrap_or(false),
            )),
            "compaction.summary" => {
                history.clear();
                history.push(ModelMessage::user(content()));
            }
            _ => {}
        }
    }
    let mut messages = vec![ModelMessage::system(system)];
    messages.extend(history);
    messages
}
pub fn builtin_tools() -> Vec<ToolDefinition> {
    fn def(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        }
    }
    let s = json!({"type":"string"});
    vec![
        def(
            "read_artifact",
            "读取当前会话附件或完整工具输出，按字节偏移分块，单次最多32768字节；UTF-8文本或base64二进制",
            json!({"hash":s,"offset":{"type":"integer","minimum":0},"length":{"type":"integer","minimum":1,"maximum":32768}}),
            &["hash", "offset", "length"],
        ),
        def(
            "read",
            "读取隔离工作副本中的UTF-8文件",
            json!({"path":s}),
            &["path"],
        ),
        def(
            "write",
            "写入副本文件。已有文件必须先read，需用户审批",
            json!({"path":s,"content":s}),
            &["path", "content"],
        ),
        def(
            "edit",
            "唯一文本替换，必须先read，需用户审批",
            json!({"path":s,"oldText":s,"newText":s}),
            &["path", "oldText", "newText"],
        ),
        def("list_files", "列出副本文件，遵守ignore规则", json!({}), &[]),
        def(
            "search",
            "按字面字符串搜索文件",
            json!({"query":s}),
            &["query"],
        ),
        def(
            "shell",
            "在Windows系统写入沙箱内运行PowerShell命令，需审批",
            json!({"command":s}),
            &["command"],
        ),
        def(
            "web_fetch",
            "获取HTTP(S)文本，需审批",
            json!({"url":s}),
            &["url"],
        ),
        def(
            "todo_write",
            "记录待办清单",
            json!({"items":{"type":"array","items":{"type":"object"}}}),
            &["items"],
        ),
        def("plan_write", "记录计划", json!({"content":s}), &["content"]),
        def("goal_write", "记录目标", json!({"content":s}), &["content"]),
        def(
            "agent_spawn",
            "创建独立子代理并立即返回会话ID；可选modelId指定已配置模型，缺省继承当前模型",
            json!({"task":s,"title":s,"modelId":s}),
            &["task"],
        ),
        def(
            "agent_status",
            "列出当前会话创建的子代理状态",
            json!({}),
            &[],
        ),
        def(
            "agent_interrupt",
            "中断当前会话创建的子代理",
            json!({"sessionId":s}),
            &["sessionId"],
        ),
        def(
            "lsp_diagnostics",
            "查询语言服务器诊断；path缺省返回全工作区缓存，wait=true时等待诊断推送并静默约3秒（冷启动首次分析可能无结果，可稍后重试）",
            json!({"path":s,"wait":{"type":"boolean"}}),
            &[],
        ),
        def(
            "lsp_hover",
            "查询语言服务器悬停信息（类型签名与文档）；line/character为1-based",
            json!({"path":s,"line":{"type":"integer","minimum":1},"character":{"type":"integer","minimum":1}}),
            &["path", "line", "character"],
        ),
        def(
            "lsp_definition",
            "查询符号定义位置；line/character为1-based",
            json!({"path":s,"line":{"type":"integer","minimum":1},"character":{"type":"integer","minimum":1}}),
            &["path", "line", "character"],
        ),
        def(
            "lsp_references",
            "查找符号全部引用；line/character为1-based",
            json!({"path":s,"line":{"type":"integer","minimum":1},"character":{"type":"integer","minimum":1},"includeDeclaration":{"type":"boolean"}}),
            &["path", "line", "character"],
        ),
        def(
            "lsp_symbols",
            "文档符号（给path）或工作区符号搜索（给query，按首个已配置语言服务器执行）",
            json!({"path":s,"query":s}),
            &[],
        ),
    ]
}
async fn fetch(url: &str, cancel: CancellationToken) -> AppResult<String> {
    let parsed = reqwest::Url::parse(url).map_err(internal)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::new("URL_SCHEME", "仅支持HTTP(S)"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(internal)?;
    let response = tokio::select! {_ =cancel.cancelled()=>return Err(cancelled()),r=client.get(parsed).send()=>r.map_err(internal)?.error_for_status().map_err(internal)?};
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) =
        tokio::select! {_=cancel.cancelled()=>return Err(cancelled()),c=stream.next()=>c}
    {
        bytes.extend_from_slice(&chunk.map_err(internal)?);
        if bytes.len() > 1024 * 1024 {
            return Err(AppError::new("FETCH_LIMIT", "响应超过1MiB"));
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into())
}
struct ReplicaHost;
impl HostPolicy for ReplicaHost {
    fn http_get_cancelled(
        &self,
        _: &str,
        url: &str,
        cancel: CancellationToken,
    ) -> AppResult<String> {
        crate::plugin_ui::http_get(url, cancel)
    }
    fn read_file(&self, _: &str, root: &Path, path: &str) -> AppResult<String> {
        harness_platform::read_file(root, path)
    }
    fn write_file(&self, _: &str, root: &Path, path: &str, contents: &str) -> AppResult<()> {
        let _mutation = FILE_MUTATION.lock().unwrap();
        harness_platform::write_file(root, path, contents)
    }
}
