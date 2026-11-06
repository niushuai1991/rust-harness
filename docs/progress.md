# 执行记录

对应 [总体实施计划](specs/implementation-plan.md)，规范入口见 [Spec 文档索引](spec-index.md)。

## 预检
- 工作目录为空且不是已有Git仓库；已初始化feat/rust-harness，无需创建嵌套worktree。
- Task 1的contracts供平台、插件、runtime、桌面共同使用，禁止各任务私改wire字段。
- Task 2平台服务由runtime调用；Task 3插件注册的工具仍经过runtime审批。
- Ruling: 初始化项目直接放在用户指定空目录 — 没有现有分支和文件需要隔离。
- 验证证据和未完成项随实现持续记录，不将占位符计为交付。

## 2026-11-04 继续执行
- 工作副本/Windows隔离13项测试；WASM/安装/MCP18项测试；前端12项测试，在各自任务验证通过。
- 独立审阅7项Important已逐一修复，并补充红→绿回归：预览hash绑定、ignore误删除、导入事务、tool批次/未知结果、UI串工作区、插件依赖升级。
- 继续整合时发现TLS加密provider特征合并冲突，显式选择ring；全工作区测试重新运行。
- Ruling: 本机已安装工具链名stable、实际版本1.98.1，使用stable并保留MSRV，不重复下载同版本工具链。
- Ruling: npm+package-lock为唯一前端锁文件；pnpm不作为第二套锁定来源。
- Ruling: Windows integration test须嵌入Common Controls v6清单；桌面/类型构建增加栈空间，防复杂命令类型构建栈溢出。
- CLI中断前只有空main，继续后补为真实stdio JSON-RPC及SDK。ACP等尚未实现，不作完成声明。
- 状态真源：capability-matrix.md。全量日志：.cache/all-tests.log；安装构建：.cache/nsis-build.log。

## 最终整合验证
- `cargo test --workspace --offline`：51项通过，包括Common Controls清单与Specta真实导出。
- `npm test`：12项交互测试通过；`npm run build`通过；Python SDK真实子进程调用通过。
- `cargo clippy --workspace --all-targets --offline`成功，有collapsible_if/参数数量等风格警告，未宣称零警告。
- `cargo fmt --all -- --check`和`git diff --check`通过。
- Specta rc.25内联serde_json::Value导致导出无限递归，已用命名递归JsonValue及透明JsonPayload修复；扩大栈本身未解决，现测试可正常完成。
- NSIS系统缓存rename返回跨盘错误，切换官方useLocalToolsDir选项解决，不修改全局缓存配置。
- 发布版真机smoke发现未配置updater时初始化报错，改为signed-updates显式功能后，发布版启动/第二实例退出测试通过。
- 标准NSIS产物成功生成，约12.56MiB；未运行用户机器上的安装器，未声称完成干净VM安装验收。
- 离线包另用直连微软运行时构建，不使用付费代理大流量。最终状态见delivery.md。
- 离线NSIS实际生成成功：218.57MiB；标准包12.56MiB，两个版本分别保留。发布版启动与单实例smoke通过。

## 0.1.1 安装权限修复
- 用户反馈选D:\Program Files时无法写入exe。旧NSIS为currentUser，实际PE权限级别asInvoker，不会自动对失败的文件写入请求UAC。
- Ruling: 用户明确要求受保护目录安装时提权，改用官方perMachine模式，在安装开始请求管理员权限，卸载注册写HKLM；应用本身不提升权限。
- 旧标准包PE回归先失败（asInvoker），新标准包已验证requireAdministrator，主程序验证asInvoker。
- 增加PE资源验收脚本并接入Windows CI/build脚本，版本统一升为0.1.1。未实际写入用户Program Files或修改其安装注册表。
- 0.1.1标准/离线包均已实际构建，PE验证全部通过：安装器requireAdministrator，应用asInvoker。

## 0.1.2 双供应商、多协议接入

- 两个内置供应商，协议作为供应商内部选项；API Key 与代理按供应商共享，复制独立凭据引用。
- 三协议统一历史、实际请求快照、工具调用和结果、流式终态、签名/加密推理回放与跨路由隔离已实现；桌面、headless、调度接入共享配置服务。
- schema 1→2 在线备份与事务迁移，失败后保留备份并允许重试；旧配置不按地址自动归为内置。
- RED→GREEN 证据覆盖多行 SSE、截断、迁移重试、协议引用保护、复制兼容规则、未闭合 Anthropic 块、Responses 正文输入、模型错误分类及 SDK 管道关闭。
- 独立审查的三个问题已修复并复核。全工作区 75 项 Rust、23 项前端、2 项 Python SDK 真实子进程测试通过；TypeScript SDK 独立类型检查与生产构建通过；Clippy 成功但保留风格警告。
- 两个 0.1.2 NSIS 包实际生成并验证 requireAdministrator，应用 asInvoker；桌面启动和单实例验证通过。标准 12.71 MiB，离线 218.71 MiB。
- 定时器回归测试取消后台 ticker，仅手动驱动 tick，以消除测试内部竞争；未修改业务调度策略。
- 实际供应商账号联调、正式签名和干净 VM 安装/卸载未验证；完整原始路线的未实现能力保持原状态。

## 后续安装包策略调整

- 按用户要求，默认目标系统已有 WebView2；Tauri 使用 skip 模式，不内嵌、下载或安装运行时。
- 移除离线配置、npm 命令、构建脚本开关及 CI 分支；CI 仅上传默认 NSIS 安装包。
- 既有 0.1.2 安装包和校验文件保留为历史交付，不重新打包或变更版本号。
- 验证通过：本地 Tauri skip 配置定义、Tauri info 配置解析、PowerShell 构建脚本语法、CI YAML 及产物过滤规则；本次未重新构建安装包。

## 0.1.3 工作区首次会话输入修复

- 根因：打开无历史会话的工作区只加载空列表，sessionId 始终为空，输入框一直 disabled。
- 空工作区现在自动创建首个会话；已有工作区继续选择已有会话。准备期间显示状态，失败后显示错误并允许手动新建重试。
- 同工作区在途创建按 Promise 去重，切走或返回时复用；过期结果不修改当前工作区状态。
- 5项新增回归先失败、修复后通过；App交互14项、前端全套28项及TypeScript/Vite生产构建通过。独立审查通过。
- 0.1.3单一NSIS已实际生成（10.99 MiB），使用系统已有WebView2的skip模式；安装器requireAdministrator和应用asInvoker清单验证通过。用户旧版进程仍在运行，未中断旧会话，未执行修复版原生进程冒烟。

## ACP 协议接口（P3，2026-11-06）

- 依据 [ACP 协议接口计划](specs/acp-plan.md)：`harness acp` 新子命令，Agent Client Protocol v1 agent 端，类型与 ndjson 帧处理来自官方 `agent-client-protocol` 5.0.0（`=5.0.0` 精确锁定 + `stdio` feature；Rust 1.98 编译通过）。桌面端与 JSON-RPC 约定不变。
- 并发设计：`session/prompt` 经 SDK `cx.spawn` 移出 dispatch loop 并将 Responder 移入延迟应答，保证回合进行中 `session/cancel` 可达；EOF 后 `cancel_all()`。
- 映射：`session/new {cwd}` 每次新建隔离工作副本与内部会话；Delta→`agent_message_chunk`/`agent_thought_chunk`；tool.call/result→`tool_call`/`tool_call_update`（kind 按工具分类，rawInput=arguments）；审批策略即 modes（ask/auto/never，set_mode 可切换）——ask 转 `session/request_permission`（allow/reject 一次），cancelled 结果视为拒绝，auto/never 直接结算；模型选择 `--model` > `cli.default_model`；`session/load` 按 seq 回放 user/assistant/tool/compaction 事件后应答。
- 终态：completed→`end_turn`、CANCELLED→`cancelled`（取消经模型调用错误返回，按规范捕获）、STEP_LIMIT→`max_turn_requests`、其余错误→`-32603` 带原消息。
- TDD：16 项映射/fixture 单测（先红后绿；官方 SDK 样例 JSON 锁定 wire 形态——权限结果为内部标签嵌套 `{"outcome":{"outcome":...}}`，pending 状态缺省序列化）；11 项 duplex 集成测试（initialize 门禁与协商、未知方法 -32601、set_mode 通知、mock SSE 真回路流式与 end_turn、无模型/不支持内容块错误、审批 ask 双路径、取消、load 有序回放、子进程 Stdio 冒烟）。
- 全套验证：fmt/clippy(-D warnings)/全工作区 test/build 通过。真实模型端到端（SQLite 快照独立数据目录 + 凭据管理器复用）：真实回合经 read 工具转述文件内容，18 段流式 chunk + end_turn；进程重启后 session/load 回放顺序正确。
- 边界：mcpServers 参数忽略（stderr 警告）；fs/terminal/elicitation/authenticate 未实现（本 Agent 不调用客户端方法，authMethods 空）；Zed 实测待用户抽查；单宿主 host.lock 约束与 rpc 相同（桌面运行时需独立 --data-dir）。

## 内置浏览器集成（2026-11-06）

- 依据[内置浏览器集成计划](specs/browser-plan.md)：新 crate `harness-browser`，注册 browser_navigate/snapshot/click/type/press/screenshot/evaluate/close 八个工具；统一命令契约 + `BrowserBackend` 抽象（参考 ZCode browser-use 三层架构），默认后端为系统 Edge/Chrome headless + 自研轻量 CDP 客户端（tokio-tungstenite，仅 Page/Runtime/Input 域）；CLI 与桌面端在 `orchestration::install` 之后接入 `harness_browser::install`。
- Ruling: Edge/Chrome（Windows）launcher 进程转交真身后立即退出，stderr 的 `DevTools listening` 行不可依赖——直接 spawn 版本子目录真身同样立即退出；就绪检测改为轮询 user-data-dir 下 `DevToolsActivePort` 文件（首行端口），进程树清理改用 Job Object（KILL_ON_JOB_CLOSE，spawn 后立即 AssignProcessToJobObject，真身继承 Job），实测关闭 Job 句柄后浏览器进程树全部终止。
- Ruling: 可执行文件解析顺序为显式指定 > 环境变量 RUST_HARNESS_BROWSER_EXECUTABLE > Edge/Chrome 标准安装路径，命中后优先切换到版本子目录中的真身可执行文件；不自动下载浏览器，未找到时报 BROWSER_NOT_FOUND。
- 安全边界：导航 scheme 白名单（仅 http/https/about:blank）；快照 ref→CSS 定位映射仅存 Rust 侧，注入脚本为一次性 IIFE 不写入页面 globalThis；浏览器使用独立临时 user-data-dir，不携带用户配置；browser_* 工具默认走审批（与 shell 同级）。
- TDD：单元测试 8 项（URL 白名单、候选路径、版本子目录 stub 解析、命令契约）先红后绿；工具链路 2 项（审批通过后触达注入的假后端、拒绝不触达）；集成测试 1 项经本地 HTTP 页面驱动真实 Edge headless 走完 navigate→snapshot→type→click→evaluate→screenshot→close。
- 全套验证：cargo fmt --check 通过；clippy 对 harness-browser 全目标零警告；cargo test --all-features 全绿；cargo build --all-features 通过。分支已 rebase 到 main（LSP PR #11 合并后）重新验证。
- 真实模型端到端（SQLite 快照独立数据目录 + 凭据管理器复用，glm-5.3-flash，--approval auto）：模型自主完成 navigate→snapshot（识别输入框/按钮 ref）→type(张三)→click→evaluate，读到"你好，张三"并在回复中报告，turn completed。
- 边界：桌面可视浏览器面板、CLI 调试子命令、多标签页管理未实现（计划范围外）；CI 无浏览器环境时集成测试打印原因跳过（不失败）；CLI 侧 `ContentChunk` 未使用导入告警为 main 基线遗留，与本改动无关。

## 子代理可视化与 spawn 模型选择（2026-11-06）

- 依据 [子代理可视化与 spawn 模型选择计划](specs/subagent-visibility-plan.md)：核实子代理协作原语与重启恢复已闭环后，补齐"用户看不见子代理"与"spawn 无法指定模型"两个缺口。
- 契约：`SessionInfo` 新增运行时推导的 `parent_id`（存储层恒空）；`Runtime` 维护 child→parent 反向映射（与既有 children 重建同源，fork 的 historyOnly 排除语义保持），新增 `parent_of`/`list_sessions`/`active_ids`。
- `agent_spawn` 可选 `modelId`：先经 `build_model` 校验再创建子会话（失败不留空会话），`subagent.created` 事件附带 `modelId`；审批要求与 8 个子代理上限不变。桌面新增 `session_active_list` 命令、CLI 新增 `session.active`，`session.list`/桌面 `session_list` 装饰父子关系。
- 前端：会话列表子任务徽标与缩进、`session_active_list` 并入 2 秒轮询显示运行 live-dot；`EventCard` 渲染 `subagent.created`/`subagent.message_sent`（可跳转子会话）与 `todo.state`/`plan.state`/`goal.state` 卡片。
- TDD 证据：`subagent_flow.rs` 3 项先红后绿（无效 modelId 拒绝不留子会话、合法 modelId 事件与登记、重启后父子关系重建与列表装饰）；前端 38 项测试通过（新增 3 项），tsc 与 Vite 生产构建通过。测试自身发现并修复：`agent_spawn` 审批未处理导致 600 秒超时（自动批准）、批准助手强引用阻止 host.lock 释放（改 Weak + 重试打开）。
- 边界：ACP/JSON-RPC 不透传子代理生命周期；子代理工具白名单、深度树管理面板、Todo/Plan/Goal 独立管理面板、会话搜索/fork 前端入口均未实现，保持原状态。
