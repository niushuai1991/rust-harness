//! Scoped agent ownership, durable tool settlements, and human approvals.
use crate::model::{MessageRole, ModelAnswer, ModelMessage, ModelProvider, StreamDelta, cancelled};
use crate::token_budget::{
    BudgetSnapshot, BudgetStop, DEFAULT_LIMIT_TOKENS, LIMIT_SETTING_KEY, TurnBudget,
    consumed_tokens, total_input_tokens, validate_limit,
};
use harness_contracts::*;
use harness_extensions::{HostPolicy, PluginManager};
use harness_platform::{ShellService, WorkspaceManager};
use harness_storage::Store;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

type PendingApproval = (ApprovalRequest, oneshot::Sender<bool>);
static FILE_MUTATION: Mutex<()> = Mutex::new(());
/// 工具批次并发调度开关（KV，缺省开启）。
const TOOL_CONCURRENCY_KEY: &str = "app.tool_concurrency";
/// 工具批次并发上限（KV，缺省 8，允许 1–16）。
const TOOL_CONCURRENCY_LIMIT_KEY: &str = "app.tool_concurrency_limit";
const DEFAULT_TOOL_CONCURRENCY_LIMIT: u32 = 8;
/// 请求前自动压缩开关（KV，缺省开启）。
const AUTO_COMPACTION_KEY: &str = "app.auto_compaction";
/// 自动压缩触发阈值：总输入 token 达到窗口的 80% 时在下一次请求前压缩。
/// 预留隐含在 20% 余量中；该默认值未经基准验证，规范要求后续测量后调整。
const AUTO_COMPACTION_THRESHOLD_RATIO: f64 = 0.8;
/// 计划模式 system 追加提示（投影重建时重复使用）。
const PLAN_MODE_HINT: &str = "\n\n当前处于计划模式：只能只读调查，不得产生项目或外部副作用。shell 工具仅允许只读命令（git 查询、文件查看与列表、搜索、系统信息、--version/--help），写文件、重定向、安装、构建、提权命令会被拒绝。请使用 plan_write 记录计划。若需要开始执行，请调用 plan_exit 并等待用户明确批准；用户拒绝或无人在场时继续只读。";

/// 工具并发上限合法性：必须有界（规范要求并发上限受控），上限 16 足够覆盖本地开发场景。
fn validate_concurrency_limit(limit: u64) -> AppResult<u32> {
    if !(1..=16).contains(&limit) {
        return Err(AppError::new(
            "TOOL_CONCURRENCY_LIMIT",
            "工具并发上限必须在 1–16 之间",
        ));
    }
    Ok(limit as u32)
}
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

type PendingQuestion = (AskRequest, oneshot::Sender<AskAnswer>);
/// ask_user 提问的挂起等待表；pending 全局共享，子代理提问按 `AskRequest.session_id` 路由到宿主。
#[derive(Default)]
pub struct Questions {
    pending: Mutex<HashMap<String, PendingQuestion>>,
}
impl Questions {
    pub fn list(&self) -> Vec<AskRequest> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .map(|(r, _)| r.clone())
            .collect()
    }
    pub fn resolve(&self, id: &str, answer: AskAnswer) -> AppResult<()> {
        let (_, tx) = self
            .pending
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| AppError::new("ASK_EXPIRED", "提问不存在或已过期"))?;
        tx.send(answer)
            .map_err(|_| AppError::new("ASK_EXPIRED", "调用已结束"))
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
    pub questions: Questions,
    /// 无人值守模式：ask_user 提问不经挂起，直接以「无人在场」自动结算（非交互宿主设置）。
    unattended: std::sync::atomic::AtomicBool,
    approval_policies: Mutex<HashMap<String, ApprovalPolicy>>,
    default_approval_policy: Mutex<Option<ApprovalPolicy>>,
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
/// 回合执行上下文：执行根、执行模式、shell 沙箱 capability 派生 id 与 worktree 模式的分支名。
#[derive(Clone)]
struct ExecContext {
    root: PathBuf,
    mode: ExecutionMode,
    capability_id: String,
    branch: Option<String>,
}
/// 单个工具调用的执行输入；owned 结构使并发段成员可独立 `tokio::spawn`。
struct ToolExecution {
    session: String,
    workspace: String,
    exec: Arc<ExecContext>,
    call: ToolCall,
    reads: Arc<Mutex<HashMap<String, String>>>,
    provider: Arc<dyn ModelProvider>,
    token_budget: Arc<TurnBudget>,
    cancel: CancellationToken,
}
/// 批次执行上下文：steps 循环内共享的借用集合，避免长参数列表。
struct CallCtx<'a> {
    session: &'a str,
    workspace: &'a str,
    exec: &'a ExecContext,
    reads: &'a Arc<Mutex<HashMap<String, String>>>,
    provider: &'a Arc<dyn ModelProvider>,
    token_budget: &'a Arc<TurnBudget>,
    cancel: &'a CancellationToken,
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
            questions: Questions::default(),
            unattended: std::sync::atomic::AtomicBool::new(false),
            approval_policies: Mutex::new(HashMap::new()),
            default_approval_policy: Mutex::new(None),
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
    /// 设置无人值守模式：ask_user 提问不经挂起，直接以「无人在场」自动结算（非交互宿主设置）。
    pub fn set_unattended(&self, unattended: bool) {
        self.unattended
            .store(unattended, std::sync::atomic::Ordering::Relaxed);
    }
    /// 无人值守模式当前状态。
    pub fn unattended(&self) -> bool {
        self.unattended.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn active_count(&self) -> usize {
        self.active.lock().unwrap().len()
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.active.lock().unwrap().contains_key(id)
    }
    /// 设置会话审批策略（权限模式）；`None` 清除，回落运行时默认或宿主交互裁决。
    pub fn set_approval_policy(&self, session: &str, policy: Option<ApprovalPolicy>) {
        let mut policies = self.approval_policies.lock().unwrap();
        match policy {
            Some(policy) => {
                policies.insert(session.to_owned(), policy);
            }
            None => {
                policies.remove(session);
            }
        }
    }
    /// 设置运行时默认审批策略：未显式设置会话策略时生效（含子代理继承与定时任务会话）。
    pub fn set_default_approval_policy(&self, policy: Option<ApprovalPolicy>) {
        *self.default_approval_policy.lock().unwrap() = policy;
    }
    /// 运行时默认审批策略。
    pub fn default_approval_policy(&self) -> Option<ApprovalPolicy> {
        *self.default_approval_policy.lock().unwrap()
    }
    /// 会话生效的审批策略：会话显式设置优先，其次运行时默认；两者皆无返回 `None`（宿主交互裁决）。
    pub fn approval_policy(&self, session: &str) -> Option<ApprovalPolicy> {
        self.approval_policies
            .lock()
            .unwrap()
            .get(session)
            .copied()
            .or_else(|| *self.default_approval_policy.lock().unwrap())
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
    /// 会话列表并附加父子关系；`workspace` 过滤工作区，`archived` 过滤归档状态（None=仅活跃）。
    pub async fn list_sessions(
        &self,
        workspace: Option<&str>,
        archived: Option<bool>,
    ) -> AppResult<Vec<SessionInfo>> {
        let mut sessions = self.store.sessions_filtered(workspace, archived).await?;
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
    fn session_queue(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.queues
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
    pub async fn session_boundary(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.session_queue(id).lock_owned().await
    }
    /// Return the durable mode, including for archived, forked and imported sessions.
    pub async fn session_plan_get(&self, session: &str) -> AppResult<bool> {
        self.store.session(session).await?;
        let events = self.store.events(session, 0).await?;
        Ok(events
            .iter()
            .rev()
            .find(|e| e.kind == "session.plan")
            .map(|e| e.data["enabled"].as_bool().unwrap_or(true))
            .unwrap_or(false))
    }
    /// Refuse to enter while the session is busy or has unverified previous work.
    pub async fn session_plan_set(&self, session: &str, enabled: bool) -> AppResult<()> {
        let _boundary = self
            .session_queue(session)
            .try_lock_owned()
            .map_err(|_| AppError::new("PLAN_MODE_BUSY", "会话正在运行，不能切换计划模式"))?;
        let info = self.store.session(session).await?;
        if self.session_plan_get(session).await? == enabled {
            return Ok(());
        }
        if enabled {
            let workspace = self.workspaces.get(&info.workspace_id)?;
            if workspace.execution_mode == ExecutionMode::Worktree
                && !workspace.worktrees.contains_key(session)
            {
                return Err(AppError::new(
                    "PLAN_MODE_WORKTREE",
                    "只读模式不能创建会话 worktree",
                ));
            }
            // Older turns have no proof their hooks and external tools left no detached work.
            let mut verified_plan = false;
            for event in self.store.events(session, 0).await? {
                if event.kind == "session.plan" {
                    verified_plan = event.data["enabled"].as_bool().unwrap_or(true);
                } else if !verified_plan
                    && matches!(
                        event.kind.as_str(),
                        "turn.start"
                            | "tool.call"
                            | "tool.result"
                            | "job.started"
                            | "terminal.opened"
                            | "schedule.created"
                            | "subagent.created"
                            | "plugin.command.started"
                            | "plugin.command.result"
                    )
                {
                    return Err(AppError::new(
                        "PLAN_MODE_UNVERIFIED",
                        "已有回合可能遗留后台工作，请使用新会话",
                    ));
                }
            }
            if !self.children_of(session).is_empty() {
                return Err(AppError::new(
                    "PLAN_MODE_UNVERIFIED",
                    "存在既有子代理，请使用新会话",
                ));
            }
        }
        self.append(session, "session.plan", json!({"enabled":enabled}))
            .await?;
        Ok(())
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
                // worktree 执行模式不建隔离副本，沙箱 stdio 反推租约失败；v1 明确不支持。
                if info.execution_mode == ExecutionMode::Worktree {
                    return Err(AppError::new(
                        "UNSUPPORTED",
                        "worktree 执行模式暂不支持 stdio MCP 服务器",
                    ));
                }
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
        // worktree 执行模式不建隔离副本，沙箱 stdio 反推租约失败；v1 明确不支持。
        if info.execution_mode == ExecutionMode::Worktree {
            return Err(AppError::new(
                "UNSUPPORTED",
                "worktree 执行模式暂不支持生产模式启动语言服务器；可用 test seam 注册连接",
            ));
        }
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
    pub(crate) fn run_session_with_budget(
        self: Arc<Self>,
        session_id: String,
        text: String,
        provider: Arc<dyn ModelProvider>,
        token_budget: Arc<TurnBudget>,
    ) -> futures::future::BoxFuture<'static, AppResult<()>> {
        Box::pin(async move {
            let message_id = self.enqueue_message(&session_id, text, vec![]).await?;
            self.execute_message_with_budget(session_id, message_id, provider, Some(token_budget))
                .await
        })
    }
    pub async fn token_budget_limit(&self) -> AppResult<u64> {
        let stored = self.store.get(LIMIT_SETTING_KEY).await?;
        match stored {
            None => Ok(DEFAULT_LIMIT_TOKENS),
            Some(value) => {
                let limit = value.as_u64().ok_or_else(|| {
                    AppError::new("TOKEN_BUDGET_LIMIT", "已保存的 Token 预算无效，请重新设置")
                })?;
                validate_limit(limit)
            }
        }
    }
    pub async fn set_token_budget_limit(&self, limit: u64) -> AppResult<()> {
        let limit = validate_limit(limit)?;
        self.store.set(LIMIT_SETTING_KEY, &json!(limit)).await
    }
    /// 工具批次并发调度开关（缺省开启；关闭时整批次回退串行）。
    pub async fn tool_concurrency_enabled(&self) -> AppResult<bool> {
        Ok(self
            .store
            .get(TOOL_CONCURRENCY_KEY)
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }
    pub async fn set_tool_concurrency(&self, enabled: bool) -> AppResult<()> {
        self.store.set(TOOL_CONCURRENCY_KEY, &json!(enabled)).await
    }
    /// 工具批次并发上限（缺省 8，允许 1–16）。
    pub async fn tool_concurrency_limit(&self) -> AppResult<u32> {
        let stored = self.store.get(TOOL_CONCURRENCY_LIMIT_KEY).await?;
        match stored {
            None => Ok(DEFAULT_TOOL_CONCURRENCY_LIMIT),
            Some(value) => {
                let limit = value.as_u64().ok_or_else(|| {
                    AppError::new(
                        "TOOL_CONCURRENCY_LIMIT",
                        "已保存的工具并发上限无效，请重新设置",
                    )
                })?;
                validate_concurrency_limit(limit)
            }
        }
    }
    pub async fn set_tool_concurrency_limit(&self, limit: u32) -> AppResult<()> {
        let limit = validate_concurrency_limit(u64::from(limit))?;
        self.store
            .set(TOOL_CONCURRENCY_LIMIT_KEY, &json!(limit))
            .await
    }
    /// 请求前自动压缩开关（缺省开启）。
    pub async fn auto_compaction_enabled(&self) -> AppResult<bool> {
        Ok(self
            .store
            .get(AUTO_COMPACTION_KEY)
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }
    pub async fn set_auto_compaction(&self, enabled: bool) -> AppResult<()> {
        self.store.set(AUTO_COMPACTION_KEY, &json!(enabled)).await
    }
    async fn publish_token_budget(
        &self,
        session: &str,
        token_budget: &TurnBudget,
        snapshot: BudgetSnapshot,
    ) -> AppResult<()> {
        let status = match snapshot.stopped {
            None => "active",
            Some(BudgetStop::LimitReached) => "exhausted",
            Some(BudgetStop::UsageUnavailable) => "usageUnavailable",
        };
        let data = json!({
            "turnId": token_budget.turn_id(),
            "usedTokens": snapshot.used_tokens,
            "limitTokens": snapshot.limit_tokens,
            "status": status,
        });
        self.append(token_budget.root_session_id(), "token.budget", data.clone())
            .await?;
        if session != token_budget.root_session_id() {
            self.append(session, "token.budget", data).await?;
        }
        Ok(())
    }
    async fn record_token_usage(
        &self,
        session: &str,
        token_budget: &TurnBudget,
        usage: &Value,
    ) -> AppResult<BudgetSnapshot> {
        let _updates = token_budget.serialize_updates().await;
        let (snapshot, usage_error) = match consumed_tokens(usage) {
            Ok(tokens) => (token_budget.add_usage(tokens), None),
            Err(error) => (token_budget.mark_usage_unavailable(), Some(error)),
        };
        if snapshot.stopped.is_some() {
            token_budget.cancel_others(session);
        }
        self.publish_token_budget(session, token_budget, snapshot)
            .await?;
        if let Some(error) = usage_error {
            return Err(token_budget.error().unwrap_or(error));
        }
        Ok(snapshot)
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
        self.execute_message_with_budget(session_id, message_id, provider, None)
            .await
    }
    async fn execute_message_with_budget(
        self: Arc<Self>,
        session_id: String,
        message_id: String,
        provider: Arc<dyn ModelProvider>,
        inherited_budget: Option<Arc<TurnBudget>>,
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
        // worktree 执行模式：会话首个 turn 前懒建宿主代建的会话 worktree
        // （写主仓 .git 由宿主完成，不经 agent 沙箱）；失败则本回合报错终止。
        let exec = self
            .resolve_exec_root(&session.workspace_id, &session_id)
            .await?;
        // 租约持有至回合结束（序列化同一工作区的并发运行与同步）。
        let _lease = self.workspaces.acquire(&session.workspace_id)?;
        let token_budget = match inherited_budget {
            Some(budget) => budget,
            None => Arc::new(TurnBudget::new(
                session_id.clone(),
                message_id.clone(),
                self.token_budget_limit().await?,
            )),
        };
        let cancel = CancellationToken::new();
        token_budget.register_session(session_id.clone(), cancel.clone());
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
            {
                let _updates = token_budget.serialize_updates().await;
                self.publish_token_budget(&session_id, &token_budget, token_budget.snapshot())
                    .await?;
            }
            self.load_skill_mentions(&session_id, &text, &exec.root)
                .await?;
            self.steps(
                &session_id,
                &session.workspace_id,
                &exec,
                provider,
                cancel.clone(),
                token_budget.clone(),
            )
            .await
        }
        .await;
        self.active.lock().unwrap().remove(&session_id);
        token_budget.unregister_session(&session_id);
        let result = match (result, token_budget.error()) {
            (Err(_), Some(budget_error)) => Err(budget_error),
            (result, _) => result,
        };
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
        exec: &ExecContext,
        provider: Arc<dyn ModelProvider>,
        cancel: CancellationToken,
        token_budget: Arc<TurnBudget>,
    ) -> AppResult<()> {
        let root = exec.root.as_path();
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
        let context = workspace_context(root, exec, &skills_catalog)?;
        self.append(session, "context.snapshot", json!({"content":context}))
            .await?;
        // read-before-edit 状态：并发段成员共享（read 登记为良性竞争），写类工具只在串行屏障执行。
        let reads = Arc::new(Mutex::new(HashMap::new()));
        let mut step = 0usize;
        // 请求前自动压缩每回合最多尝试一次：失败不无限重试，避免反复请求溢出。
        let mut auto_compaction_tried = false;
        loop {
            if cancel.is_cancelled() {
                return Err(token_budget.error().unwrap_or_else(cancelled));
            }
            token_budget.check_available()?;
            let mut tools = builtin_tools();
            if !self.session_plan_get(session).await? {
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
            }
            let context_hooks = if self.session_plan_get(session).await? {
                Vec::new()
            } else {
                self.plugins
                    .run_hook(
                        workspace,
                        root,
                        "before_request",
                        json!({"sessionId":session,"step":step}),
                        cancel.clone(),
                    )
                    .await?
            };
            for context in context_hooks {
                self.append(
                    session,
                    "plugin.context",
                    json!({"content":context["context"]}),
                )
                .await?;
            }
            let mut history = self.store.events(session, 0).await?;
            let mut messages = project_messages(&history);
            let plan_mode = self.session_plan_get(session).await?;
            if plan_mode {
                messages[0].content.push_str(PLAN_MODE_HINT);
            }
            let request_id = new_id();
            // —— 请求前自动压缩：输入接近窗口阈值时，先压缩投影再继续原请求 ——
            if !auto_compaction_tried
                && self.auto_compaction_enabled().await?
                && let Some(window) = provider.context_window()
                && let Some(input) = last_input_tokens(&history)?
            {
                auto_compaction_tried = true;
                let threshold = (f64::from(window) * AUTO_COMPACTION_THRESHOLD_RATIO) as u64;
                if input >= threshold {
                    match self
                        .compact_projection(
                            session,
                            provider.clone(),
                            cancel.clone(),
                            CompactionTrigger::Auto,
                        )
                        .await
                    {
                        Ok(answer) => {
                            // 压缩请求的 usage 计入回合共享预算（不得通过压缩绕过预算）。
                            self.record_token_usage(session, &token_budget, &answer.usage)
                                .await?;
                            token_budget.check_available()?;
                            // 基于摘要重建投影后继续原请求。
                            history = self.store.events(session, 0).await?;
                            messages = project_messages(&history);
                            if plan_mode {
                                messages[0].content.push_str(PLAN_MODE_HINT);
                            }
                        }
                        Err(error) => {
                            if input >= u64::from(window) {
                                // 压缩失败且继续请求必然溢出：给出明确可诊断结果。
                                self.append(
                                    session,
                                    "assistant.attempt",
                                    json!({"requestId":request_id,"error":error}),
                                )
                                .await?;
                                return Err(AppError::new(
                                    "CONTEXT_WINDOW_EXCEEDED",
                                    format!("上下文已达模型窗口上限且自动压缩失败：{error}"),
                                ));
                            }
                            // 仍有余量：不替换有效投影，本回合不再重试。
                            self.append(
                                session,
                                "assistant.attempt",
                                json!({"requestId":request_id,"error":format!("自动压缩失败：{error}")}),
                            )
                            .await?;
                        }
                    }
                }
            }
            // Tool discovery, hooks and history reads can await while a
            // sibling consumes the shared budget. Recheck at the provider-call boundary.
            token_budget.check_available()?;
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
            let partial_usage = Arc::new(Mutex::new(None::<Value>));
            let partial_usage_for_sink = partial_usage.clone();
            let partial_usage_sink = Arc::new(move |usage| {
                *partial_usage_for_sink.lock().unwrap() = Some(usage);
            });
            let answer = match provider
                .complete_with_partial_usage(
                    messages,
                    tools.clone(),
                    cancel.clone(),
                    sink,
                    partial_usage_sink,
                )
                .await
            {
                Ok(answer) => answer,
                Err(e) => {
                    let partial_usage =
                        partial_usage.lock().unwrap().take().filter(|usage| {
                            e.code != "CANCELLED" || consumed_tokens(usage).is_ok()
                        });
                    let error = if let Some(usage) = partial_usage {
                        match self
                            .record_token_usage(session, &token_budget, &usage)
                            .await
                        {
                            Ok(_) => e,
                            Err(error) => error,
                        }
                    } else {
                        e
                    };
                    let p = partial.lock().unwrap().clone();
                    self.append(
                        session,
                        "assistant.attempt",
                        json!({"requestId":request_id,"content":p.0,"reasoning":p.1,"error":error}),
                    )
                    .await?;
                    return Err(token_budget.error().unwrap_or(error));
                }
            };
            let budget_snapshot = match self
                .record_token_usage(session, &token_budget, &answer.usage)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.append(
                        session,
                        "assistant.attempt",
                        json!({"requestId":request_id,"content":answer.content,"reasoning":answer.reasoning,"error":error}),
                    )
                    .await?;
                    return Err(error);
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
            if budget_snapshot.stopped.is_some() {
                let error = token_budget.error().unwrap_or_else(|| {
                    AppError::new("TOKEN_BUDGET_EXCEEDED", "本轮 Token 预算已耗尽")
                });
                for call in answer.calls {
                    self.append(
                        session,
                        "tool.result",
                        json!({
                            "callId": call.id,
                            "name": call.name,
                            "content": format!("{error}；工具未执行。"),
                            "isError": true,
                            "status": "denied",
                            "artifact": null,
                        }),
                    )
                    .await?;
                }
                return Err(error);
            }
            // 工具批次执行：白名单成员的连续段并发执行（上限受控），其余保持串行。
            // 段内 prepare（校验/钩子）保持串行语义；结果按声明顺序结算，保证事件回放一致。
            let concurrency = self.tool_concurrency_enabled().await?;
            let concurrency_limit = self.tool_concurrency_limit().await? as usize;
            let ctx = CallCtx {
                session,
                workspace,
                exec,
                reads: &reads,
                provider: &provider,
                token_budget: &token_budget,
                cancel: &cancel,
            };
            let mut queue: VecDeque<ToolCall> = answer.calls.into_iter().collect();
            while let Some(first) = queue.pop_front() {
                let first_groupable = concurrency
                    && self.batch_concurrency_ok(&first.name)
                    && self.batch_approval_free(&first.name);
                if first_groupable {
                    // 收集连续白名单段；非白名单成员即屏障，不跨越重排。
                    let mut segment = vec![first];
                    while let Some(next) = queue.front() {
                        if self.batch_concurrency_ok(&next.name)
                            && self.batch_approval_free(&next.name)
                        {
                            segment.push(queue.pop_front().expect("front 已确认非空"));
                        } else {
                            break;
                        }
                    }
                    // 段内逐个串行 prepare（钩子可变状态安全），全部完成后统一执行与结算。
                    let mut prepared = Vec::with_capacity(segment.len());
                    let mut broken = false;
                    for call in segment {
                        let prepared_call = self
                            .prepare_call(call, &tools, session, workspace, root, &cancel)
                            .await?;
                        if prepared_call.failure.is_none()
                            && (prepared_call.needs_approval
                                || !self.batch_concurrency_ok(&prepared_call.call.name))
                        {
                            // 钩子改写或改写后需审批：整段退回串行（复用已完成的 prepare，不重复钩子）。
                            broken = true;
                        }
                        prepared.push(prepared_call);
                    }
                    if broken || prepared.len() < 2 {
                        for prepared_call in prepared {
                            let (call, result) = self.execute_prepared(prepared_call, &ctx).await;
                            self.settle_call(&call, result, session, workspace, root, &cancel)
                                .await?;
                        }
                    } else {
                        // 并发执行：Semaphore 限流；全部完成后按声明顺序结算。
                        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency_limit));
                        let mut handles = Vec::with_capacity(prepared.len());
                        for (index, prepared_call) in prepared.iter().enumerate() {
                            if prepared_call.failure.is_some() {
                                continue; // 准备期已失败的成员不执行，结算时直接落错误结果。
                            }
                            let runtime = self.clone();
                            let semaphore = semaphore.clone();
                            let input = ToolExecution {
                                session: ctx.session.to_owned(),
                                workspace: ctx.workspace.to_owned(),
                                exec: Arc::new(ctx.exec.clone()),
                                call: prepared_call.call.clone(),
                                reads: ctx.reads.clone(),
                                provider: ctx.provider.clone(),
                                token_budget: ctx.token_budget.clone(),
                                cancel: ctx.cancel.clone(),
                            };
                            handles.push((
                                index,
                                tokio::spawn(async move {
                                    let _permit =
                                        semaphore.acquire().await.expect("并发上限信号量不会关闭");
                                    run_tool_guarded(runtime, input).await
                                }),
                            ));
                        }
                        let mut outcomes: HashMap<usize, AppResult<ToolResult>> = HashMap::new();
                        for (index, handle) in handles {
                            outcomes.insert(index, handle.await.map_err(internal)?);
                        }
                        for (index, prepared_call) in prepared.into_iter().enumerate() {
                            let result = match outcomes.remove(&index) {
                                Some(result) => result,
                                None => Err(prepared_call
                                    .failure
                                    .clone()
                                    .expect("未执行成员必有准备期失败")),
                            };
                            self.settle_call(
                                &prepared_call.call,
                                result,
                                session,
                                workspace,
                                root,
                                &cancel,
                            )
                            .await?;
                        }
                    }
                } else {
                    // 串行完整路径：prepare → execute（含审批发起）→ settle，与既有行为一致。
                    let prepared_call = self
                        .prepare_call(first, &tools, session, workspace, root, &cancel)
                        .await?;
                    let (call, result) = self.execute_prepared(prepared_call, &ctx).await;
                    self.settle_call(&call, result, session, workspace, root, &cancel)
                        .await?;
                }
            }
            step = step.saturating_add(1);
        }
    }
    /// 解析会话执行根与执行上下文：replica 模式为隔离副本（capability 派生自工作区 id）；
    /// worktree 模式为宿主代建的会话 worktree（capability 派生自会话 id，与 prepare 授权一致）。
    async fn resolve_exec_root(
        &self,
        workspace_id: &str,
        session_id: &str,
    ) -> AppResult<ExecContext> {
        let info = self.workspaces.get(workspace_id)?;
        if info.execution_mode == ExecutionMode::Worktree
            && self.session_plan_get(session_id).await?
            && !info.worktrees.contains_key(session_id)
        {
            return Err(AppError::new(
                "PLAN_MODE_WORKTREE",
                "只读模式不能创建会话 worktree",
            ));
        }
        if info.execution_mode == ExecutionMode::Replica {
            return Ok(ExecContext {
                root: PathBuf::from(&info.replica_path),
                mode: ExecutionMode::Replica,
                capability_id: workspace_id.to_owned(),
                branch: None,
            });
        }
        let manager = self.workspaces.clone();
        let wid = workspace_id.to_owned();
        let sid = session_id.to_owned();
        let worktree =
            tokio::task::spawn_blocking(move || manager.ensure_session_worktree(&wid, &sid))
                .await
                .map_err(internal)??;
        Ok(ExecContext {
            root: PathBuf::from(&worktree.path),
            mode: ExecutionMode::Worktree,
            capability_id: session_id.to_owned(),
            branch: Some(worktree.branch),
        })
    }
    pub(crate) async fn approve(
        &self,
        session: &str,
        call: &ToolCall,
        cancel: &CancellationToken,
        escalated: bool,
        denial_rule: Option<&str>,
    ) -> AppResult<()> {
        let policy = self.approval_policy(session);
        // 自主档下 shell 危险命令在审批理由中点名规则，便于用户判断。
        let dangerous = (policy == Some(ApprovalPolicy::Guarded) && call.name == "shell")
            .then(|| {
                call.arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            })
            .and_then(dangerous_shell_rule);
        let request = ApprovalRequest {
            id: new_id(),
            session_id: session.into(),
            tool: call.name.clone(),
            arguments: call.arguments.clone(),
            escalated,
            reason: match (escalated, dangerous) {
                (true, _) => match denial_rule {
                    Some(rule) => {
                        format!("命令疑似被沙箱拦截（{rule}）。是否不带沙箱重跑一次？")
                    }
                    None => "该命令请求不带沙箱直接执行。是否批准一次提权重跑？".into(),
                },
                (false, Some(rule)) => {
                    format!("自主模式检测到危险命令（{rule}），需要确认本次执行。")
                }
                (false, None) => "允许此工具在隔离工作副本中执行？原项目仍需单独确认同步。".into(),
            },
        };
        // 会话级策略（桌面端设置）可自动裁决的档位不经 pending 等待；事件流仍完整记录请求与结算。
        if let Some(allowed) = policy.and_then(|p| p.decide(&request)) {
            self.append(
                session,
                "approval.request",
                serde_json::to_value(&request).map_err(internal)?,
            )
            .await?;
            self.append(
                session,
                "approval.resolved",
                json!({"id":request.id,"allow":allowed}),
            )
            .await?;
            if allowed {
                return Ok(());
            }
            return Err(AppError::new(
                "APPROVAL_DENIED",
                "会话审批策略拒绝此工具调用",
            ));
        }
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
    /// ask_user：向用户提问并挂起等待答复（免审批白名单工具）。
    ///
    /// 无人在场（[`Runtime::set_unattended`]）或超过 [`ASK_WAIT_SECS`] 秒自动结算，
    /// 均返回「请自行决策」答复让回合继续；取消时落结算事件保持请求/结算配对完整。
    /// ask_user：向用户提问并挂起等待答复，返回面向模型的文本结果。
    async fn ask_user(
        &self,
        session: &str,
        a: &Value,
        cancel: &CancellationToken,
    ) -> AppResult<String> {
        let (request, answer) = self.ask_user_answer(session, a, cancel).await?;
        Ok(answer_text(&answer, &request))
    }
    async fn ask_user_answer(
        &self,
        session: &str,
        a: &Value,
        cancel: &CancellationToken,
    ) -> AppResult<(AskRequest, AskAnswer)> {
        let request = AskRequest {
            id: new_id(),
            session_id: session.into(),
            question: a["question"].as_str().unwrap_or_default().into(),
            options: a["options"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let label = item["label"].as_str()?;
                            Some(AskOption {
                                label: label.into(),
                                description: item["description"].as_str().map(Into::into),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let payload = serde_json::to_value(&request).map_err(internal)?;
        // 无人在场：不经挂起直接结算，事件流仍完整记录请求与结算（同审批策略直结算路径）。
        if self.unattended() {
            let answer = unattended_answer();
            self.append(session, "ask.request", payload).await?;
            self.append(
                session,
                "ask.resolved",
                resolved_value(&request.id, &answer),
            )
            .await?;
            return Ok((request, answer));
        }
        let (tx, rx) = oneshot::channel();
        self.questions
            .pending
            .lock()
            .unwrap()
            .insert(request.id.clone(), (request.clone(), tx));
        let logged = self.append(session, "ask.request", payload).await;
        if logged.is_err() {
            self.questions.pending.lock().unwrap().remove(&request.id);
            logged.map(|_| ())?;
        }
        let answer = tokio::select! {
            _ = cancel.cancelled() => {
                // 回合取消：仍落结算事件保持配对完整；结果随回合取消收尾。
                Some(AskAnswer { text: "回合已取消，未获得答复".into(), source: AskSource::Timeout })
            }
            r = rx => Some(r.unwrap_or_else(|_| AskAnswer { text: "用户未能答复".into(), source: AskSource::Timeout })),
            _ = tokio::time::sleep(std::time::Duration::from_secs(ASK_WAIT_SECS)) => Some(unattended_answer()),
        };
        self.questions.pending.lock().unwrap().remove(&request.id);
        let answer = answer.expect("select 分支均产出答复");
        self.append(
            session,
            "ask.resolved",
            resolved_value(&request.id, &answer),
        )
        .await?;
        Ok((request, answer))
    }
    async fn execute_tool(self: &Arc<Self>, input: ToolExecution) -> AppResult<ToolResult> {
        let ToolExecution {
            session,
            workspace,
            exec,
            call,
            reads,
            provider,
            token_budget,
            cancel,
        } = input;
        let root = exec.root.as_path();
        let a = &call.arguments;
        // edit/write 的 unified diff（tool.result 事件展示用）；其余工具保持 None。
        let mut diff: Option<String> = None;
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
                        .artifact_read(&session, arg(a, "hash")?, offset, length)
                        .await?,
                )
                .map_err(internal)?
            }
            "read" => {
                let path = arg(a, "path")?;
                let root = root.to_path_buf();
                let path_for_io = path.to_owned();
                let value =
                    blocking_io(move || harness_platform::read_file(&root, &path_for_io)).await?;
                reads.lock().unwrap().insert(path.into(), value.clone());
                value
            }
            "write" => {
                let path = arg(a, "path")?.to_owned();
                let content = arg(a, "content")?.to_owned();
                let root = root.to_path_buf();
                let reads = reads.clone();
                let worktree = exec.mode == ExecutionMode::Worktree;
                let (text, write_diff) = blocking_io(move || {
                    // 文件写入互斥：与并发段隔离（写类工具只在串行屏障执行）。
                    let _mutation = FILE_MUTATION.lock().unwrap();
                    let existing = harness_platform::resolve_path(&root, &path, false)?;
                    let before = if existing.exists() {
                        let current = harness_platform::read_file(&root, &path)?;
                        if reads.lock().unwrap().get(&path) != Some(&current) {
                            return Err(AppError::new(
                                "READ_BEFORE_EDIT",
                                "修改已有文件前必须读取当前版本",
                            ));
                        }
                        Some(current)
                    } else {
                        None
                    };
                    harness_platform::write_file(&root, &path, &content)?;
                    reads.lock().unwrap().insert(path.clone(), content.clone());
                    let write_diff =
                        harness_platform::diffview::from_texts(&path, before.as_deref(), &content);
                    Ok::<_, AppError>((
                        format!("已写入{} {path}", if worktree { "" } else { "副本" }),
                        write_diff,
                    ))
                })
                .await?;
                diff = write_diff;
                text
            }
            "edit" => {
                let path = arg(a, "path")?.to_owned();
                let old = arg(a, "oldText")?.to_owned();
                let new = arg(a, "newText")?.to_owned();
                let root = root.to_path_buf();
                let reads = reads.clone();
                let worktree = exec.mode == ExecutionMode::Worktree;
                let (text, edit_diff) = blocking_io(move || {
                    let _mutation = FILE_MUTATION.lock().unwrap();
                    if !reads.lock().unwrap().contains_key(&path) {
                        return Err(AppError::new("READ_BEFORE_EDIT", "edit前必须read"));
                    }
                    let content = harness_platform::read_file(&root, &path)?;
                    if reads.lock().unwrap().get(&path) != Some(&content) {
                        return Err(AppError::new(
                            "EDIT_CONFLICT",
                            "读取后文件已改变，请重新read",
                        ));
                    }
                    if old.is_empty() || content.matches(&old).count() != 1 {
                        return Err(AppError::new(
                            "EDIT_CONFLICT",
                            "待替换文本必须在当前文件中恰好出现一次",
                        ));
                    }
                    let updated = content.replacen(&old, &new, 1);
                    harness_platform::write_file(&root, &path, &updated)?;
                    reads.lock().unwrap().insert(path.clone(), updated.clone());
                    let edit_diff =
                        harness_platform::diffview::from_texts(&path, Some(&content), &updated);
                    Ok::<_, AppError>((
                        format!("已修改{} {path}", if worktree { "" } else { "副本" }),
                        edit_diff,
                    ))
                })
                .await?;
                diff = edit_diff;
                text
            }
            "list_files" => {
                let root = root.to_path_buf();
                serde_json::to_string(
                    &blocking_io(move || harness_platform::list_files(&root)).await?,
                )
                .map_err(internal)?
            }
            "search" => {
                let query = arg(a, "query")?.to_owned();
                let root = root.to_path_buf();
                blocking_io(move || {
                    let mut found = Vec::new();
                    for path in harness_platform::list_files(&root)?.into_iter().take(10000) {
                        if let Ok(text) = harness_platform::read_file(&root, &path) {
                            for (i, line) in text.lines().enumerate() {
                                if line.contains(&query) {
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
                    Ok::<_, AppError>(found.join("\n"))
                })
                .await?
            }
            "shell" => {
                let command = arg(a, "command")?;
                let plan_mode = self.session_plan_get(&session).await?;
                let mut escalated = false;
                let output = if plan_mode {
                    // 计划模式只读命令：单次沙箱内执行，失败按普通失败呈现，不申请提权重跑。
                    ShellService::run_in(root, &exec.capability_id, command, cancel.clone(), true)
                        .await?
                } else if a["escalate"].as_bool().unwrap_or(false) {
                    // 主动提权：首次即审批，批准后无沙箱执行。
                    self.approve(&session, &call, &cancel, true, None).await?;
                    escalated = true;
                    ShellService::run_in(root, &exec.capability_id, command, cancel.clone(), false)
                        .await?
                } else {
                    let first = ShellService::run_in(
                        root,
                        &exec.capability_id,
                        command,
                        cancel.clone(),
                        true,
                    )
                    .await?;
                    // 沙箱拒绝归类：命中关键词则申请提权重跑一次，裁决走统一审批通道。
                    if first.exit_code != 0
                        && let Some(rule) = harness_platform::classify_sandbox_denial(&first)
                    {
                        self.approve(&session, &call, &cancel, true, Some(rule))
                            .await?;
                        escalated = true;
                        ShellService::run_in(
                            root,
                            &exec.capability_id,
                            command,
                            cancel.clone(),
                            false,
                        )
                        .await?
                    } else {
                        first
                    }
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&output).map_err(internal)?,
                    is_error: output.exit_code != 0,
                    diff: None,
                    escalated,
                });
            }
            "web_fetch" => fetch(arg(a, "url")?, cancel).await?,
            "todo_write" | "plan_write" | "goal_write" => {
                self.append(
                    &session,
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
                        &workspace,
                        a.get("title").and_then(Value::as_str).unwrap_or("子任务"),
                    )
                    .await?;
                self.children
                    .lock()
                    .unwrap()
                    .entry(session.clone())
                    .or_default()
                    .push(child.id.clone());
                self.parents
                    .lock()
                    .unwrap()
                    .insert(child.id.clone(), session.clone());
                // 子代理继承发起会话的审批策略，避免策略在子代理处失效。
                if let Some(policy) = self.approval_policy(&session) {
                    self.set_approval_policy(&child.id, Some(policy));
                }
                let mut created = json!({"sessionId":child.id});
                if let Some(id) = model_id {
                    created["modelId"] = json!(id);
                }
                self.append(&session, "subagent.created", created).await?;
                let runtime = self.clone();
                let id = child.id.clone();
                let task = arg(a, "task")?.to_owned();
                tokio::spawn(async move {
                    if let Err(e) = runtime
                        .run_session_with_budget(id, task, provider, token_budget)
                        .await
                    {
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
                    .get(&session)
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
                    .get(&session)
                    .is_some_and(|ids| ids.iter().any(|x| x == id))
                {
                    return Err(AppError::new("AGENT_SCOPE", "只能控制本会话创建的子代理"));
                }
                self.cancel(id);
                "已请求中断".into()
            }
            "ask_user" => self.ask_user(&session, a, &cancel).await?,
            "plan_exit" => {
                if !self.session_plan_get(&session).await? {
                    return Err(AppError::new("PLAN_MODE_INACTIVE", "当前不在计划模式"));
                }
                let question = json!({
                    "question": format!("是否批准以下计划并退出只读模式？\n\n{}", arg(a, "content")?),
                    "options": [{"label":"批准"},{"label":"拒绝"}]
                });
                let (request, answer) = self.ask_user_answer(&session, &question, &cancel).await?;
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                // 只有「选项选择 + 精确等于批准」才算批准；自由输入、无人值守、超时均不算。
                let approved =
                    matches!(answer.source, AskSource::Selected) && answer.text == "批准";
                if !approved {
                    return Err(AppError::new(
                        "PLAN_NOT_APPROVED",
                        format!("计划未获明确批准；{}", answer_text(&answer, &request)),
                    ));
                }
                // The turn owns session_boundary, so no external mode transition can race this commit.
                self.append(&session, "session.plan", json!({"enabled":false}))
                    .await?;
                "用户已批准计划并退出计划模式。".into()
            }
            name if name.starts_with("plugin__") => {
                return self
                    .plugins
                    .execute_tool(&workspace, root, name, a.clone(), cancel)
                    .await;
            }
            name if name.starts_with("mcp__") => {
                return self.mcp.execute_tool(name, a.clone(), cancel).await;
            }
            name if name.starts_with("lsp_") => {
                return Ok(ToolResult {
                    content: self
                        .execute_lsp_tool(name, &workspace, root, a, cancel)
                        .await?,
                    is_error: false,
                    diff: None,
                    escalated: false,
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
                            session_id: session.clone(),
                            workspace_id: workspace.clone(),
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
            diff,
            escalated: false,
        })
    }
    /// 批次并发安全判定：内置白名单或外部工具的显式并发能力声明。
    fn batch_concurrency_ok(&self, name: &str) -> bool {
        concurrency_safe_tool(name)
            || self
                .custom
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|t| t.concurrency_safe())
    }
    /// 审批豁免判定（与执行点判定同源）：并发段成员必须免审批。
    fn batch_approval_free(&self, name: &str) -> bool {
        !self
            .custom
            .lock()
            .unwrap()
            .get(name)
            .map(|t| t.requires_approval())
            .unwrap_or_else(|| requires_approval(name))
    }
    /// 批次执行前的准备：定义查找、schema 校验、before_tool 钩子（含 rewrite 落库）、
    /// 计划模式判定与审批需求判定（不发起审批）。校验类错误记入 `failure`，
    /// 在结算阶段按既有错误语义落 tool.result；钩子可变状态操作保持串行调用。
    async fn prepare_call(
        &self,
        mut call: ToolCall,
        tools: &[ToolDefinition],
        session: &str,
        workspace: &str,
        root: &Path,
        cancel: &CancellationToken,
    ) -> AppResult<PreparedCall> {
        let outcome = async {
            let definition = tools
                .iter()
                .find(|t| t.name == call.name)
                .ok_or_else(|| AppError::new("TOOL_UNAVAILABLE", &call.name))?;
            let schema = jsonschema::validator_for(&definition.parameters).map_err(internal)?;
            if !schema.is_valid(&call.arguments) {
                return Err(AppError::new("TOOL_ARGUMENTS", "工具参数不符合JSON Schema"));
            }
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let original = serde_json::to_value(&call).map_err(internal)?;
            if !self.session_plan_get(session).await? {
                for rewrite in self
                    .plugins
                    .run_hook(
                        workspace,
                        root,
                        "before_tool",
                        original.clone(),
                        cancel.clone(),
                    )
                    .await?
                {
                    call.name = rewrite["name"]
                        .as_str()
                        .ok_or_else(|| AppError::new("PLUGIN_HOOK", "缺少工具名称"))?
                        .into();
                    call.arguments = rewrite["arguments"].clone();
                }
            }
            if original != serde_json::to_value(&call).map_err(internal)? {
                self.append(
                    session,
                    "tool.rewrite",
                    json!({"callId": call.id, "original": original, "effective": call}),
                )
                .await?;
            }
            let effective = tools
                .iter()
                .find(|t| t.name == call.name)
                .ok_or_else(|| AppError::new("TOOL_UNAVAILABLE", &call.name))?;
            if !jsonschema::validator_for(&effective.parameters)
                .map_err(internal)?
                .is_valid(&call.arguments)
            {
                return Err(AppError::new("TOOL_ARGUMENTS", "钩子修改后的工具参数无效"));
            }
            let plan_mode = self.session_plan_get(session).await?;
            if plan_mode && !plan_tool_allowed(&call.name, &call.arguments) {
                return Err(AppError::new(
                    "PLAN_MODE_READ_ONLY",
                    plan_denial_reason(&call.name, &call.arguments),
                ));
            }
            // 计划模式走到这里的工具都已被白名单判定为只读（内置只读清单或只读 shell 命令），
            // 无需再走审批；提权请求已在白名单处拒绝，不存在待批事项。
            let needs_approval = !plan_mode
                && self
                    .custom
                    .lock()
                    .unwrap()
                    .get(&call.name)
                    .map(|t| t.requires_approval())
                    .unwrap_or_else(|| requires_approval(&call.name));
            Ok(needs_approval)
        }
        .await;
        match outcome {
            Ok(needs_approval) => Ok(PreparedCall {
                call,
                needs_approval,
                failure: None,
            }),
            Err(error) => Ok(PreparedCall {
                call,
                needs_approval: false,
                failure: Some(error),
            }),
        }
    }
    /// 执行已准备的调用：审批发起（若需要）→ 子 token 取消/超时守卫下的工具执行。
    /// 返回调用本体（结算需要 id/name/arguments）与执行结果。
    async fn execute_prepared(
        self: &Arc<Self>,
        prepared: PreparedCall,
        ctx: &CallCtx<'_>,
    ) -> (ToolCall, AppResult<ToolResult>) {
        let PreparedCall {
            call,
            needs_approval,
            failure,
        } = prepared;
        if let Some(error) = failure {
            return (call, Err(error));
        }
        if needs_approval
            && let Err(error) = self
                .approve(ctx.session, &call, ctx.cancel, false, None)
                .await
        {
            return (call, Err(error));
        }
        let result = run_tool_guarded(
            self.clone(),
            ToolExecution {
                session: ctx.session.to_owned(),
                workspace: ctx.workspace.to_owned(),
                exec: Arc::new(ctx.exec.clone()),
                call: call.clone(),
                reads: ctx.reads.clone(),
                provider: ctx.provider.clone(),
                token_budget: ctx.token_budget.clone(),
                cancel: ctx.cancel.clone(),
            },
        )
        .await;
        (call, result)
    }
    /// 结果结算：状态判定、后置钩子、溢出 blob 与 tool.result 事件落库（语义与既有串行路径一致）。
    async fn settle_call(
        &self,
        call: &ToolCall,
        result: AppResult<ToolResult>,
        session: &str,
        workspace: &str,
        root: &Path,
        cancel: &CancellationToken,
    ) -> AppResult<()> {
        let status = match &result {
            Ok(_) => "completed",
            Err(e) if matches!(e.code.as_str(), "CANCELLED" | "TOOL_TIMEOUT") => "unknown",
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "APPROVAL_DENIED" | "PLAN_MODE_READ_ONLY" | "PLAN_NOT_APPROVED"
                ) =>
            {
                "denied"
            }
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
                diff: None,
                escalated: false,
            },
        };
        if !cancel.is_cancelled() && !self.session_plan_get(session).await? {
            match self
                .plugins
                .run_hook(
                    workspace,
                    root,
                    "after_tool",
                    json!({"call": call, "result": result}),
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
        let result_diff = result.diff;
        // edit/write 结果事件带上文件路径（结果事件无 arguments，前端展示行级差异卡需要）。
        let result_path = if call.name.as_str() == "edit" || call.name.as_str() == "write" {
            call.arguments.get("path").cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        };
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
            content =
                format!("{preview}\n\n[完整输出 artifact:{hash}；使用 read_artifact 分块读取]");
        }
        self.append(
            session,
            "tool.result",
            json!({
                "callId": call.id,
                "name": call.name,
                "content": content,
                "isError": result.is_error,
                "status": status,
                "artifact": artifact,
                "diff": result_diff,
                "path": result_path,
                "escalated": result.escalated,
            }),
        )
        .await?;
        Ok(())
    }
    /// 压缩会话投影：清洗 replay/reasoning → 摘要请求 → 校验 → 落 `compaction.summary`。
    /// 原始事件保留；投影在 compaction.summary 处重置（见 [`project_messages`]）。
    /// 返回原始回答供调用方记账（自动压缩计入回合预算，手动压缩由宿主决定语义）。
    pub async fn compact_projection(
        &self,
        session: &str,
        provider: Arc<dyn ModelProvider>,
        cancel: CancellationToken,
        trigger: CompactionTrigger,
    ) -> AppResult<ModelAnswer> {
        let events = self.store.events(session, 0).await?;
        let mut messages = project_messages(&events);
        for message in &mut messages {
            message.replay = None;
            message.reasoning.clear();
        }
        let request = vec![
            ModelMessage::system(
                "请用中文总结会话目标、已完成工作、关键事实、未完成事项和工具执行结果。保留继续任务所需信息，不编造。",
            ),
            ModelMessage::user(serde_json::to_string(&messages).map_err(internal)?),
        ];
        let answer = provider
            .complete(request, vec![], cancel.clone(), Arc::new(|_| {}))
            .await?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if answer.content.trim().is_empty() || !answer.calls.is_empty() {
            return Err(AppError::new(
                "COMPACTION_INVALID",
                "模型没有返回有效纯文本摘要",
            ));
        }
        self.append(
            session,
            "compaction.summary",
            json!({
                "content": answer.content,
                "throughSeq": events.last().map(|e| e.seq.clone()),
                "trigger": trigger.as_str(),
            }),
        )
        .await?;
        Ok(answer)
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
/// 同步文件 IO 移入 tokio 阻塞线程池：并发段的磁盘读取/目录遍历真正多线程并行，
/// 且不挤占 async 工作线程（串行单工具路径同样受益）。
async fn blocking_io<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(internal)?
}
/// 单个调用的准备结果：钩子/校验完成后的调用、审批需求与准备期失败。
struct PreparedCall {
    call: ToolCall,
    needs_approval: bool,
    failure: Option<AppError>,
}
/// 工具执行守卫：子 token 取消、执行时限与工具体竞速；取消/超时先中断并等待收尾。
async fn run_tool_guarded(runtime: Arc<Runtime>, input: ToolExecution) -> AppResult<ToolResult> {
    let is_ask = input.call.name == "ask_user";
    let tool_cancel = input.cancel.child_token();
    let turn_cancel = input.cancel.clone();
    let input = ToolExecution {
        cancel: tool_cancel.clone(),
        ..input
    };
    let work = runtime.execute_tool(input);
    tokio::pin!(work);
    // ask_user 挂起等待用户答复，与审批同享 600 秒等待上限；其余工具维持 180 秒执行时限。
    let tool_limit = if is_ask { ASK_WAIT_SECS } else { 180 };
    tokio::select! {
        _ = turn_cancel.cancelled() => {tool_cancel.cancel(); let _ = work.await; Err(cancelled())},
        _ = tokio::time::sleep(std::time::Duration::from_secs(tool_limit)) => {tool_cancel.cancel(); let _ = work.await; Err(AppError::new("TOOL_TIMEOUT", "工具超过执行时限"))},
        r = &mut work => r,
    }
}
/// ask_user 挂起等待上限（秒）：与审批 600 秒兜底一致。
const ASK_WAIT_SECS: u64 = 600;
/// 无人在场/超时的统一自动结算答复：提问失败不摧毁回合。
fn unattended_answer() -> AskAnswer {
    AskAnswer::unattended()
}
/// ask.resolved 事件 data。
fn resolved_value(id: &str, answer: &AskAnswer) -> Value {
    json!({"id":id,"answer":answer.text,"source":answer.source})
}
/// 给模型的 tool result 文本：区分选项选择、自由输入与无人在场结算。
fn answer_text(answer: &AskAnswer, request: &AskRequest) -> String {
    match answer.source {
        AskSource::Selected => {
            let description = request
                .options
                .iter()
                .find(|o| o.label == answer.text)
                .and_then(|o| o.description.as_deref())
                .map(|d| format!("（{d}）"))
                .unwrap_or_default();
            format!("用户选择了「{}」{description}", answer.text)
        }
        AskSource::Text => format!("用户答复：{}", answer.text),
        AskSource::Unattended => "当前无用户在场（未答复），请基于现有信息自行决策并继续。".into(),
        AskSource::Timeout => "等待答复超时，请基于现有信息自行决策并继续。".into(),
    }
}
// This is intentionally independent of approval exemption. Unknown and third-party
// tools, even those declaring no approval, cannot execute in plan mode.
fn plan_safe_tool(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "read_artifact"
            | "list_files"
            | "search"
            | "ask_user"
            | "plan_write"
            | "plan_exit"
    )
}
/// 工具批次并发安全白名单：显式能力声明，不凭名称推断。
/// 逐项审计依据（纯只读、无外部副作用、无交互、不依赖批次内其他调用结果）：
/// - `read`：读副本文件并登记 read-before-edit 状态，登记为良性竞争（同文件并发读取内容一致）；
/// - `list_files` / `search`：只读目录遍历与内容搜索；
/// - `read_artifact`：分块读会话附件/溢出输出 blob。
///
/// LSP、MCP、插件、自定义、shell、web_fetch、写类与交互类工具均默认串行；
/// 外部工具需经 `ExternalTool::concurrency_safe` 显式声明才可进入并发段。
fn concurrency_safe_tool(name: &str) -> bool {
    matches!(name, "read" | "list_files" | "search" | "read_artifact")
}
/// 计划模式工具准入：内置只读白名单之外，shell 仅放行可静态证明只读的命令。
/// 提权请求（escalate）一律拒绝；这是独立于审批档位的硬边界。
fn plan_tool_allowed(name: &str, arguments: &Value) -> bool {
    if plan_safe_tool(name) {
        return true;
    }
    name == "shell"
        && arguments["escalate"].as_bool() != Some(true)
        && read_only_shell_command(arguments["command"].as_str().unwrap_or_default())
}
/// 拒绝文案按原因区分，帮助模型理解下一步可做什么。
fn plan_denial_reason(name: &str, arguments: &Value) -> &'static str {
    if name == "shell" && arguments["escalate"].as_bool() == Some(true) {
        "计划模式不允许提权执行 shell 命令"
    } else if name == "shell" {
        "计划模式 shell 仅允许只读命令（git 查询、文件查看与列表、搜索、系统信息），写文件、重定向、安装、构建均被拒绝"
    } else {
        "计划模式只允许安全读取和记录计划"
    }
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
            | "plan_exit"
            | "goal_write"
            | "agent_status"
            | "ask_user"
    )
}
fn workspace_context(root: &Path, exec: &ExecContext, skills_catalog: &str) -> AppResult<String> {
    let mut context = match exec.mode {
        ExecutionMode::Replica => format!(
            "你是本地开发助手。执行根为隔离工作副本 {}。文件工具只接受相对路径。先读取再编辑，原项目变更需用户在差异界面单独确认。",
            root.display()
        ),
        ExecutionMode::Worktree => format!(
            "你是本地开发助手。执行根为源项目的 worktree 分支 {}（路径 {}），你的编辑直接生效于该分支工作区。worktree 已由宿主创建，无需自行 git worktree add。主仓 .git 与 main 工作区不可直接写入，commit/push 等操作用 shell 执行 git 命令，被沙箱拦截时会触发提权审批。文件工具只接受相对路径。先读取再编辑。",
            exec.branch.as_deref().unwrap_or("?"),
            root.display()
        ),
    };
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
/// 从事件流提取最近一次模型请求的总输入 token（压缩点后重置为 None）。
/// 请求前自动压缩的阈值检查使用：上下文占用 = 全部输入（含缓存读），口径见 [`total_input_tokens`]。
pub(crate) fn last_input_tokens(events: &[SessionEvent]) -> AppResult<Option<u64>> {
    for event in events.iter().rev() {
        match event.kind.as_str() {
            "compaction.summary" => return Ok(None),
            "assistant.message" => {
                let usage = &event.data["usage"];
                if usage.is_null() {
                    continue;
                }
                return total_input_tokens(usage).map(Some);
            }
            _ => {}
        }
    }
    Ok(None)
}
/// 压缩触发来源：手动 `/compact`（宿主命令）或请求前自动压缩（阈值触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionTrigger {
    Manual,
    Auto,
}
impl CompactionTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            CompactionTrigger::Manual => "manual",
            CompactionTrigger::Auto => "auto",
        }
    }
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
            "在Windows系统写入沙箱内运行PowerShell命令，需审批；escalate为true时请求不带沙箱重跑（需审批）",
            json!({"command":s,"escalate":{"type":"boolean"}}),
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
        def(
            "ask_user",
            "向用户提问并等待答复。options 提供候选项时用户可选择其中之一或自由输入；省略 options 为纯自由文本提问。仅在缺少用户决策无法继续时使用，无人在场环境下会自动以「无人在场」结算",
            json!({"question":s,"options":{"type":"array","items":{"type":"object","properties":{"label":s,"description":s},"required":["label"],"additionalProperties":false}}}),
            &["question"],
        ),
        def("plan_write", "记录计划", json!({"content":s}), &["content"]),
        def(
            "plan_exit",
            "提交计划并请求用户明确批准；无人答复或拒绝时继续只读",
            json!({"content":s}),
            &["content"],
        ),
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
