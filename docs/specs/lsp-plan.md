# LSP 语言服务集成计划

拟制日期：2026-11-06。

状态：已实施（2026-11-06 完成，实施记录见文末；真实 rust-analyzer 沙箱冒烟通过）。

依据：用户决定将 P3 阶段中的 LSP 能力提前实施（原 P3 条目"SSH/LSP/web/document"之一）；经对能力矩阵、`harness-extensions/src/mcp.rs` 接入范式与参考仓库 `D:\code\codex` 调研后拟制本计划（codex 无 LSP 实现可借鉴），按 TDD 实施。

## 背景与目标

当前 Agent 的代码理解手段只有 `read`/`search`/`list_files` 等文本级工具，缺少语言级智能（诊断、定义、引用、符号）。本计划引入 LSP（Language Server Protocol）集成，为模型提供五个只读工具：

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

- LSP 的 JSON-RPC over stdio 采用 Content-Length 帧格式，编解码手写为 tokio_util::codec 的 Decoder/Encoder；报文按 JSON 直通处理，不引入 `lsp-types` 等重型框架。
- `LspConnection`：initialize 握手（60 秒超时）→ 请求/响应 id 匹配（并发请求各自 pending 表）→ 通知分发；`textDocument/publishDiagnostics` 推送按 URI 缓存到 `Mutex<BTreeMap<...>>`。
- **URI 盘符大小写无关匹配**：真实集成发现 rust-analyzer 回显的诊断 URI 会把 Windows 盘符规范化为小写（我们构造的 `file:///C:/...` 被回显为 `file:///c:/...`），按原样字符串匹配会导致推送缓存与等待全部落空（冒烟中表现为诊断永远为空）。修复为存储与查询统一经 `uri_key()` 做盘符大小写无关匹配；server 对此行为差异各自不同，客户端侧统一归一是正解。
- `LspManager`：仿 `McpManager`（mcp.rs:45-47）——`RwLock<BTreeMap<(workspace_id, server_id), Arc<Connection>>>` + generation `CancellationToken`。`connect()` 泛型接受 AsyncRead/AsyncWrite 管道，进程创建留在 platform 沙箱（与 mcp.rs 模块注释同一设计口径：本模块只接收管道）；server 崩溃（stdio EOF）→ 连接移除，下次调用按新 generation 自动重启；`Drop` 全量 cancel，宿主退出杀掉全部 server。
- 连接按 `(workspace_id, server_id)` 绑定，存活至 Runtime 结束（工作副本生命周期内 server 状态持续有效）。

### 配置（settings KV）

- 键 `lsp.servers.v1`：`Vec<LspServerConfig>`，字段 `id`/`command`/`args`/`extensions`；键缺省时默认 `[{ id: "rust-analyzer", command: "rust-analyzer", args: [], extensions: ["rs"] }]`。
- 类型定义放 `harness-contracts/src/lib.rs`（加 `specta::Type` 供桌面端 TS 导出），经 `Store::get/set` 读写（参考 orchestration `schedules()` 模式）。

### 运行时接线（`harness-runtime/src/agent.rs`）

- `Runtime` 增加 `lsp: Arc<LspManager>` 字段。
- `builtin_tools()` 增加五个工具定义。参数约定：
  - `path` 一律为工作副本相对路径（与 read/write 一致）；
  - `line`/`character` 为 1-based（模型友好），内部转 LSP 0-based；
  - `lsp_diagnostics`：`path`、`wait` 均可选；`path` 缺省返回全工作区缓存；`wait=true` 时等待**该文件首次诊断推送**后静默约 3 秒返回，未推送则最长等待 120 秒（实测 rust-analyzer 冷启动需加载 std 源码与 build scripts，首个推送可能滞后 1 分钟以上，轮询重试是模型可见的兜底语义）；
  - `lsp_symbols`：有 `path` 走 `textDocument/documentSymbol`，无 `path` 走 `workspace/symbol`（须给 `query`，按首个已配置语言服务器执行）。
- `execute_tool()` 增加五个 match 分支，公共流程：按文件扩展名解析 server 配置 → 未运行则 `ShellService::spawn_stdio(root=工作副本, command, args, cancel)` 拉起并握手 → 文件同步（未打开则 didOpen；已打开且磁盘内容变化则 didChange 全量推送、version 递增）→ 发请求 → 结果 JSON 序列化为 content 字符串。单请求超时 60 秒，处于既有全局 180 秒工具超时内。
- `requires_approval()` 豁免名单加入五个工具名。
- server 启动失败（如 PATH 无 rust-analyzer）返回明确错误并提示经 `lsp.servers.v1` 配置。

### 宿主 API（调试最小面）

- 桌面 `src-tauri/src/commands.rs`：`lsp_status(workspace_id)`（各 server 运行态/打开文档数/诊断数）、`lsp_restart(workspace_id, server_id)`；`#[specta::specta]` 并登记 `builder()`。
- CLI RPC dispatch：`lsp.status` / `lsp.restart` 对应方法。
- 不做前端面板；工具调用卡片自然可见。服务器状态变更不新增持久事件类型。

## 实施步骤（TDD）

1. **帧编解码**：先写失败测试（header 解析边界、拆包/粘包、非法 header、超大 Content-Length 拒绝），再实现编解码器。
2. **管理器**：进程内 mock LSP server fixture（仿 `harness-extensions/tests/mcp.rs` 的 `tokio::io::duplex` 模式）驱动失败测试——initialize 握手、didOpen/didChange 同步、publishDiagnostics 缓存、hover/definition/references/symbols 请求与并发 id 匹配、取消、EOF 崩溃清理与重启；再实现 `LspConnection`/`LspManager`。
3. **contracts**：`LspServerConfig`（specta）与默认值测试。
4. **runtime 接线**：新 `crates/harness-runtime/tests/lsp_flow.rs`（复用 agent_flow.rs 的 Recorded 假 provider 模式）——五工具分派路由、参数校验（1-based 转换、相对路径）、免审批豁免、未配置扩展名报错、server 启动失败报错；再实现 agent.rs 分支。
5. **宿主 API**：桌面两命令 + CLI RPC 两方法，登记与 TS 绑定导出回归。

## 测试计划

- 编解码：header/拆包/粘包/非法输入全覆盖。
- 管理器：上节全部场景。
- runtime：分派、参数转换、豁免名单、配置默认值、错误路径。
- 桌面命令与 CLI RPC：现有命令测试模式回归，新增 lsp.status/lsp.restart 路径。

## 验证方式

- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`。
- 本机装有 rust-analyzer 时做真实链路冒烟：工作副本内调用 `lsp_diagnostics(wait=true)` 与 `lsp_definition`，确认真实 server 可用；rust-analyzer 在 Windows 受限 token 沙箱内会再 spawn cargo/rustc，此为首要验证点，受阻则在实施记录中给出显式决策。
- 结果登记 `docs/capability-matrix.md`：LSP 拆出为独立行"已实现并接线"（附测试证据），原合并行剩余项保持未实现。

## 风险点

1. rust-analyzer 在受限 token/Job 沙箱内 spawn cargo/rustc 可能受干扰——冒烟验证结果：沙箱内 hover 与诊断均正常（E0308 诊断推送到位），spawn cargo 无阻碍；期间另发现并修复了与本风险无关的客户端 URI 盘符匹配缺陷（见"管理器"小节）。
2. rust-analyzer 首次加载慢（需加载 std 源码与 build scripts，观测 1 分钟以上）——`wait` 等首次推送 + 静默检测与 120 秒上限兜底，模型轮询重试为可见语义。
3. lsp-types 0.97 的 Uri 与 Windows 绝对路径转换需处理分隔符与百分号编码。
4. 非 Windows 平台 `spawn_stdio` 本就返回 UNSUPPORTED，LSP 工具将明确报错，与 shell 口径一致。

## 修订记录

- 2026-11-06：初版。实施中发现两处设计修正并回写：wait 语义由"静默即返回"改为"等该文件首次推送再静默"；新增 URI 盘符大小写无关匹配（真实 rust-analyzer 集成缺陷的根因修复）。

## 实施记录（2026-11-06）

分支 `feat/lsp-readonly-tools`，按 TDD 分步提交：

| 内容 | 测试 |
|---|---|
| Content-Length 帧编解码（`harness-extensions/src/lsp.rs`） | 编解码 8 项（roundtrip/拆包/粘包/大小写/非法输入） |
| `LspConnection`/`LspManager`（连接、握手、同步、推送缓存、崩溃重启） | `tests/lsp.rs` 11 项（mock server 经 duplex） |
| contracts `LspServerConfig`（含 `initializationOptions`）与默认配置 | contracts 2 项 |
| runtime 五工具接线（分派、豁免、配置、错误路径） | `tests/lsp_flow.rs` 6 项 |
| 桌面 `lsp_status`/`lsp_restart`；CLI RPC `lsp.status`/`lsp.restart` | 现有回归覆盖 |
| 真实 rust-analyzer 沙箱冒烟 | `tests/lsp_smoke.rs`（`#[ignore]`，本机显式运行通过，6.7 秒内 E0308 诊断与 hover 双到位） |

验证结果：`cargo fmt --check` 通过；涉事四 crate（contracts/extensions/runtime/cli）`cargo clippy --all-targets --all-features` 零告警；`cargo test --all-features` 29 个测试目标全绿；`cargo build --workspace --all-features` 通过。全 workspace clippy 剩余 7 条告警均为 main 既有（`rust-harness-desktop` 卸载测试 dead code），与本改动无关。

实施过程中的关键排障记录（root cause 值得留档）：真实 rust-analyzer 冒烟初期诊断持续为空，先后排除了沙箱 token 阻断、cargo check 失败、文件监视器干扰三个假设（沙箱内手工 `cargo check` 正常、裸进程诊断 2 秒到达、hover 在沙箱内正常）；最终以原始客户端逐帧记录定位根因——rust-analyzer 回显诊断 URI 时将 Windows 盘符规范化为小写（`file:///C:/…` → `file:///c:/…`），客户端按原样字符串匹配导致推送缓存与等待全部落空。修复为 `uri_key()` 盘符大小写无关匹配，修复后沙箱冒烟 6.7 秒通过。

与计划的偏差：无结构性偏差；计划中的 `lsp-types` 依赖未引入——请求构造与响应解析以 JSON 直通即可满足五件套语义，避免死依赖（后续引入工作区编辑等复杂响应解析时再加）；新增 `initialization_options` 配置字段属通用能力扩展。
