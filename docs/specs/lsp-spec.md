# LSP 语言服务集成规范

- 日期：2026-11-06（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：P3 提前项：只读五件套 LSP 工具（诊断/悬停/定义/引用/符号），通用 server 配置，默认 rust-analyzer。

## 背景与目标

Agent 的代码理解手段此前只有 `read`/`search`/`list_files` 等文本级工具，缺少语言级智能（诊断、定义、引用、符号）。本规范约定 LSP（Language Server Protocol）集成，为模型提供五个只读工具：

- `lsp_diagnostics`：文件或全工作区诊断（推送缓存）。
- `lsp_hover`：位置悬停信息（类型签名、文档）。
- `lsp_definition`：定义跳转。
- `lsp_references`：引用查找。
- `lsp_symbols`：文档符号（给 `path`）或工作区符号查询（给 `query`）。

全部只读、免审批；语言服务器（下称 server）进程经平台沙箱拉起，首次调用工具时自动启动，配置即授权。

## 范围决策（2026-11-06 用户确认）

1. **只读五件套**，不含 rename 等工作区编辑（写操作风险高，后续另行计划）。
2. **通用语言框架 + 默认 rust-analyzer**：server 命令/参数/文件扩展名映射走 settings 配置，不改代码即可接入其他语言（tsserver、pyright 等）。
3. **首次调用自动启动**：按配置在首次 LSP 工具调用时经沙箱拉起 server，按 workspace 管理，宿主退出时关闭；不做 MCP 式显式 connect 交互。

## 设计

### 管理器（`harness-extensions/src/lsp.rs`，与 mcp.rs 并列）

- LSP 的 JSON-RPC over stdio 采用 Content-Length 帧格式，编解码手写为 tokio_util::codec 的 Decoder/Encoder；报文按 JSON 直通处理，不引入 `lsp-types` 等重型框架（请求构造与响应解析以 JSON 直通满足五件套语义，后续引入工作区编辑等复杂响应解析时再加）。
- `LspConnection`：initialize 握手（60 秒超时）→ 请求/响应 id 匹配（并发请求各自 pending 表）→ 通知分发；`textDocument/publishDiagnostics` 推送按 URI 缓存到 `Mutex<BTreeMap<...>>`。
- **URI 盘符大小写无关匹配**：rust-analyzer 回显的诊断 URI 会把 Windows 盘符规范化为小写（构造的 `file:///C:/...` 被回显为 `file:///c:/...`），按原样字符串匹配会导致推送缓存与等待全部落空。存储与查询统一经 `uri_key()` 做盘符大小写无关匹配；server 对此行为差异各自不同，客户端侧统一归一是正解。
- `LspManager`：仿 `McpManager`——`RwLock<BTreeMap<(workspace_id, server_id), Arc<Connection>>>` + generation `CancellationToken`。`connect()` 泛型接受 AsyncRead/AsyncWrite 管道，进程创建留在 platform 沙箱（与 mcp.rs 模块注释同一设计口径：本模块只接收管道）；server 崩溃（stdio EOF）→ 连接移除，下次调用按新 generation 自动重启；`Drop` 全量 cancel，宿主退出杀掉全部 server。
- 连接按 `(workspace_id, server_id)` 绑定，存活至 Runtime 结束（工作副本生命周期内 server 状态持续有效）。

### 配置（settings KV）

- 键 `lsp.servers.v1`：`Vec<LspServerConfig>`，字段 `id`/`command`/`args`/`extensions`（含 `initialization_options` 通用扩展字段）；键缺省时默认 `[{ id: "rust-analyzer", command: "rust-analyzer", args: [], extensions: ["rs"] }]`。
- 类型定义放 `harness-contracts/src/lib.rs`（加 `specta::Type` 供桌面端 TS 导出），经 `Store::get/set` 读写（参考 orchestration `schedules()` 模式）。

### 运行时接线（`harness-runtime/src/agent.rs`）

- `Runtime` 增加 `lsp: Arc<LspManager>` 字段。
- `builtin_tools()` 增加五个工具定义。参数约定：
  - `path` 一律为工作副本相对路径（与 read/write 一致）；
  - `line`/`character` 为 1-based（模型友好），内部转 LSP 0-based；
  - `lsp_diagnostics`：`path`、`wait` 均可选；`path` 缺省返回全工作区缓存；`wait=true` 时等待**该文件首次诊断推送**后静默约 3 秒返回，未推送则最长等待 120 秒（rust-analyzer 冷启动需加载 std 源码与 build scripts，首个推送可能滞后 1 分钟以上，轮询重试是模型可见的兜底语义）；
  - `lsp_symbols`：有 `path` 走 `textDocument/documentSymbol`，无 `path` 走 `workspace/symbol`（须给 `query`，按首个已配置语言服务器执行）。
- `execute_tool()` 增加五个 match 分支，公共流程：按文件扩展名解析 server 配置 → 未运行则 `ShellService::spawn_stdio(root=工作副本, command, args, cancel)` 拉起并握手 → 文件同步（未打开则 didOpen；已打开且磁盘内容变化则 didChange 全量推送、version 递增）→ 发请求 → 结果 JSON 序列化为 content 字符串。单请求超时 60 秒，处于既有全局 180 秒工具超时内。
- `requires_approval()` 豁免名单加入五个工具名。
- server 启动失败（如 PATH 无 rust-analyzer）返回明确错误并提示经 `lsp.servers.v1` 配置。

### 宿主 API（调试最小面）

- 桌面 `src-tauri/src/commands.rs`：`lsp_status(workspace_id)`（各 server 运行态/打开文档数/诊断数）、`lsp_restart(workspace_id, server_id)`；`#[specta::specta]` 并登记 `builder()`。
- CLI RPC dispatch：`lsp.status` / `lsp.restart` 对应方法。
- 不做前端面板；工具调用卡片自然可见。服务器状态变更不新增持久事件类型。

## 风险与约束

1. rust-analyzer 在受限 token/Job 沙箱内 spawn cargo/rustc 正常（沙箱内 hover 与诊断均正常），沙箱兼容不构成约束。
2. rust-analyzer 首次加载慢（需加载 std 源码与 build scripts，可达 1 分钟以上）——`wait` 等首次推送 + 静默检测与 120 秒上限兜底，模型轮询重试为可见语义。
3. 非 Windows 平台 `spawn_stdio` 本就返回 UNSUPPORTED，LSP 工具将明确报错，与 shell 口径一致。
