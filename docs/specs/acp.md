# ACP 协议接口规范

- 日期：2026-11-06（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：`harness acp` 子命令：Agent Client Protocol v1 agent 端，官方 Rust SDK，供 Zed、JetBrains、Neovim 等编辑器客户端接入。

## 背景与目标

headless 此前只有自研 `harness rpc`（ndjson JSON-RPC，`harness-cli/src/lib.rs::serve`）。本规范约定 ACP（Agent Client Protocol）v1 agent 端实现：编辑器客户端可以标准方式拉起 `harness acp` 子进程并驱动本 Agent。传输为 ndjson JSON-RPC over stdio，与 v1 规范一致。

## 范围决策（2026-11-06 用户确认）

1. **官方 SDK**：引入 `agent-client-protocol = "=5.0.0"`（精确锁定；该 SDK 迭代快——5 天内 3.6→5.0，且 MSRV 未标注）。使用其 `Agent.v1()` builder、`on_receive_request`/`on_receive_notification` handler、`ConnectionTo` 发送端与 `Stdio` 传输；ndjson 帧格式由 SDK 处理，不自写协议循环。
2. **实现方法面（agent 角色，v1）**：`initialize`、`session/new`、`session/load`、`session/prompt`、`session/cancel`（通知）、`session/set_mode`；发出 `session/update`、`session/request_permission`。
3. **非目标（首版）**：`authenticate`/`logout`（`authMethods` 返回空数组，调用报 method not found）；`fs/*`、`terminal/*`、`elicitation/*`（本 Agent 从不调用客户端这些方法，无需服务端实现）；`session/resume`/`close`/`delete`/`list`（不声明相应能力）；configOptions、usage_update、available_commands_update、plan/todo 更新映射（`plan_write` 为自由文本、`todo_write` items 形状由模型自定，与 ACP 结构化 plan 条目不匹配）；`mcpServers` 参数忽略（stderr 警告；宿主 MCP 由宿主配置管理，客户端注入列为后续）。
4. **会话语义**：`session/new {cwd}` 每次 `workspaces.create` 新建隔离工作副本 + 新会话（不按路径复用旧副本，复制 `run.rs` 复用策略列为后续）；`sessionId` 直接使用内部会话 UUID（不透明字符串）。

## 设计

### 并发模型（SDK dispatch loop 约束）

- `on_receive_request`/`on_receive_notification` 回合**阻塞** SDK dispatch loop；`session/prompt` 若同步 await 整回合会卡死 `session/cancel`。
- 因此 `session/prompt` handler 通过 `cx.spawn` 把回合执行移出 dispatch loop，并将 `Responder` 移入 spawn 的任务，在回合结束后应答 `PromptResponse { stopReason }`；`Responder` 未应答即 drop 时 SDK 自动回错误，兜底安全。
- `session/cancel` 通知 handler 调 `Runtime::cancel(sessionId)`；取消语义由运行时完成（turn.end status=cancelled）。

### 会话与状态

- 连接级共享状态（`Arc` 捕获进各 handler）：`initialized` 标志（未 initialize 前的 session 方法回 `-32600`）、`model_id`（`--model` 参数）、`sessions: BTreeMap<sessionId, {workspaceId, 审批策略}>`。
- 审批策略即 ACP modes：`ask`（默认）/`auto`/`never`；`session/new`、`session/load` 响应携带 `modes`（`SessionModeState`）；`session/set_mode` 更新策略并回发 `current_mode_update`。
- 模型选择：`harness acp --model <id>` > settings `cli.default_model`（复用 `manage.rs::resolve_model_id`）；ACP v1 无 per-prompt 模型概念。`harness acp --preset` 启动解析一次预设（见[配置预设分层规范](config-presets.md)）。
- 退出（EOF/进程结束）时 `runtime.cancel_all()`，与 `serve()` 口径一致。

### 事件映射（写成纯函数，单元测试覆盖）

`runtime.subscribe()` 广播（broadcast 4096）→ 仅转发已知 sessionId：

| 运行时事件 | ACP 输出 |
|---|---|
| `LiveEvent::Delta { text }` | `session/update` `agent_message_chunk`（Text 块） |
| `LiveEvent::Delta { reasoning }` | `agent_thought_chunk` |
| `tool.call` | `tool_call`（status=pending；kind 映射：read/list_files→read、search→search、write/edit→edit、shell→execute、web_fetch→fetch、todo/plan/goal_write→think、其余→other；rawInput=arguments） |
| `tool.result` | `tool_call_update`（isError→completed/failed；content=文本块） |
| `turn.end` | 不发通知；由 prompt handler 用于 stopReason |
| `user.message`/`assistant.message`/`compaction.summary` | 仅 `session/load` 回放时使用 |

- broadcast `Lagged`：按 per-session seq 游标从 `store.events(after)` 补拉（参照 `run.rs::resync`）。
- prompt 的 stopReason：回合结束后从订阅缓冲中取该会话 `turn.end`/`Status`：completed→`end_turn`、cancelled→`cancelled`（规范要求捕获取消异常）；`run_session` 返回错误时，`STEP_LIMIT`→`max_turn_requests`，其余回 JSON-RPC `-32603`（`AppError.message`）。

### 审批 → request_permission

`approval.request`（含 id/sessionId/tool/arguments）→ 按该会话策略：
- `ask`：发 `session/request_permission`（options：`allow`/AllowOnce、`reject`/RejectOnce；`toolCall` 用该会话最近一次 pending `tool_call` 的 id——工具串行执行，`tool.call` 事件必然紧邻先行）；selected(optionId)→`approvals.resolve(id, allow_bool)`；cancelled→拒绝（与运行时取消即拒一致）。权限结果 wire 形态为内部标签嵌套 `{"outcome":{"outcome":...}}`（SDK 5.0.0 serde 形态）。
- `auto`：直接 resolve(true)；`never`：resolve(false)。600 秒运行时超时兜底不变。

### session/load 回放

校验会话存在 → 按 seq 遍历 `store.events`：`user.message`→`user_message_chunk`；`assistant.message`→`agent_message_chunk`（messageId=事件 id）+ reasoning→`agent_thought_chunk`；`tool.call`/`tool.result`→`tool_call`/`tool_call_update` 终态对；`compaction.summary`→摘要文本 `agent_message_chunk`（不声明 session.compaction 能力，不做 compaction_update）；其余 kind 忽略 → 全部发完后应答 `LoadSessionResponse { modes }`。

### 传输与入口

- 生产：`Stdio::new()`；stdout 仅输出 ACP 消息，日志走 stderr（tracing）。
- 测试：`ByteStreams::new(outgoing, incoming)`（接受 futures AsyncRead/Write；tokio DuplexStream 经 `tokio-util` compat 特性桥接，dev-dependency）。
- `main.rs` 子命令 `Commands::Acp { --model <id>, --preset <id> }`，与 `Rpc` 并列，复用 `data_dir()`、`Runtime::open`、`harness_orchestration::install`。
- 单宿主约束：`host.lock` 与桌面/`rpc` 互斥；桌面运行时 `harness acp` 须 `--data-dir` 指向独立目录（写入使用说明）。

## 风险与约束

1. SDK 迭代快——`=5.0.0` 精确锁定；升级另行评估。
2. dispatch loop 阻塞语义——prompt 必须 spawn + Responder 移入；cancel 并发须有集成测试覆盖。
3. 审批与最近 pending tool_call 的关联依赖事件顺序（tool.call 先于 approval.request）——运行时串行保证。
4. `session/new` 每次新建工作副本的成本——符合隔离语义，复用策略列为后续。
5. Zed 实测依赖用户环境——以 fixture + duplex 集成 + 脚本端到端为验收底线。
