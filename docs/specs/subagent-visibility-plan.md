# 子代理可视化与 spawn 模型选择计划

拟制日期：2026-11-06。

状态：已实施（2026-11-06 完成，实施记录见文末）。

依据：用户在 CLI、ACP、LSP 专项完成后要求继续总体计划中未做的任务，选择「子代理」方向。经核实，子代理协作原语（spawn/status/interrupt/send/wait/fork）与重启恢复（`Store::recover()` 为所有会话补 `tool.result{status:unknown}` 与 `turn.end{interrupted}`）均已闭环；真实缺口是子代理对用户不可见，以及 `agent_spawn` 无法指定模型。据此拟制本计划。

## 背景与目标

- `subagent.created`/`subagent.message_sent` 事件在前端 `EventCard` 中被静默丢弃（返回 null），用户在父会话看不到子代理的创建与投递；会话列表无父子标识，无法跳转子代理会话，也看不到其运行状态。
- Todo/Plan/Goal 的 `*.state` 事件同样被丢弃，能力矩阵记载「尚无独立管理面板」。本计划先补会话流内的状态渲染，独立管理面板留待后续。
- `agent_spawn` 只能继承父会话模型，无法为子代理指定已配置的其他模型。

## 范围决策

1. **只做可视化与 spawn 模型选择**，不改动子代理执行语义（调度、租约、审批、scope 隔离维持原状）。
2. **不做**：ACP/JSON-RPC 实时透传子代理生命周期、子代理工具白名单、深度树管理面板、Todo/Plan/Goal 独立管理面板、前端会话搜索/fork 入口。以上留待后续专项。
3. 契约演进最小化：`SessionInfo` 只增加可选 `parentId`，由运行时装饰，存储层不落库（父子关系始终从 `subagent.created` 事件推导，与 `Runtime::open` 重建逻辑同源）。

## 设计

### 契约与运行时（`harness-contracts`、`harness-runtime/src/agent.rs`）

- `SessionInfo` 增加 `parent_id: Option<String>`（serde camelCase→`parentId`；Option 缺省反序列化为 None，向后兼容）。存储层三处构造恒为 `None`。
- `Runtime` 增加 `parents: Mutex<HashMap<child, parent>>` 反向映射：
  - `Runtime::open` 重建 `children` 的既有循环同步填充（fork 副本内 `historyOnly=true` 的事件已被既有条件排除，人工 fork 不产生父子关系；`agent_fork` 写入源会话的事件无该标记，重启后关系保持）；
  - `agent_spawn` 与 `adopt_child` 同时维护两表；
  - 新增 `parent_of()`、`list_sessions(workspace)`（列表装饰 `parentId`）、`active_ids()`（当前执行回合的会话标识）。
- `agent_spawn` 增加可选 `modelId`：先经 `ModelService::build_model` 校验（MODEL_NOT_FOUND / 凭证缺失原样透传），**失败时不创建子会话**；成功后 `subagent.created` 事件 data 附带 `modelId`，后台任务使用该 provider，缺省仍继承父 provider。工具定义同步增加 `modelId` 可选参数。
- `agent_spawn` 保持需审批（不在免审批白名单），审批语义不变。

### 桌面与 CLI 接口

- 桌面 `session_list` 改用 `Runtime::list_sessions`；新增 `session_active_list` 命令（返回 `active_ids()`）。
- CLI RPC `session.list` 同样装饰；新增 `session.active` 方法。

### 前端（`frontend/`）

- `types.ts` 的 `SessionInfo` 镜像增加 `parentId?:string|null`。
- 会话列表：有 `parentId` 的会话缩进并显示「子任务」徽标；`running` 或 `session_active_list` 轮询结果（并入既有 2 秒审批轮询 effect）中的会话显示 live-dot。
- `EventCard` 新增分支（props 改为 `EventCardExtras` 透传 `onOpenSession`/`activeIds`）：
  - `subagent.created`：卡片「已创建子代理」，附「查看会话」跳转按钮（`openSession` 会话缺失时经 `session_list` 补齐并校验同工作区）；运行中显示 live-dot；`historyOnly=true`（fork 副本内的历史记录）不提供跳转并说明来源。
  - `subagent.message_sent`：「已向子代理投递任务」+ 内容预览 + 同上跳转。
  - `todo.state`：待办清单卡片（逐项渲染 text/content/title 与 done/completed/status 完成态，完成项划线）。
  - `plan.state`/`goal.state`：计划/目标文本卡片（Markdown 渲染）。
- 样式追加 `sessions button.child`、`subagent-badge`、`subagent-row`、`subagent-card`、`todo-list`。

## 测试（TDD）

- `crates/harness-runtime/tests/subagent_flow.rs`（新文件，先红后绿）：
  - 无效 `modelId` → 工具结果 `isError` 含 `MODEL_NOT_FOUND`，且不创建子会话（工作区仅父会话）；
  - 合法 `modelId`（自定义供应商副本 + 本地死端点，空密钥可构建）→ `subagent.created` 事件含 `modelId`、子会话已登记、工具结果无错；
  - `parent_of` 在 spawn 后正确；丢弃 Runtime 重开后（子任务短暂持锁，重试等待 host.lock）父子关系从事件重建正确，且 `list_sessions` 装饰 `parentId`（父会话为 None）。
  - 注意点：`agent_spawn` 需审批，测试须自动批准；批准助手持 `Weak<Runtime>`，避免阻止 host.lock 释放。
- 前端 `frontend/App.test.tsx` 新增 3 项：子代理事件卡片渲染、跳转子会话、运行状态与「子任务」徽标；fork 历史记录不提供跳转；todo/plan 状态渲染。

## 实施记录（2026-11-06）

- TDD：`subagent_flow.rs` 3 项先红（`parent_of`/`list_sessions` 编译失败确认缺失），实现后转绿（0.17s）。中途发现两处测试自身问题：未处理 `agent_spawn` 审批导致各测试等待 600 秒审批超时（自动批准解决）；批准助手持强引用阻止 host.lock 释放导致重启用例 `HOST_RUNNING`（改 Weak + 重试打开解决）。
- 前端 38 项测试通过（含新增 3 项）；`tsc --noEmit` 与 Vite 生产构建通过。发现并修复重构引入的 `workspaceId` 解构引用遗漏。
- 全套验证：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 通过（见 PR 描述）。
- 边界：`parent_id` 为运行时推导，CLI/桌面旧客户端多收到一个可为 null 的字段；`agent_spawn` 的 8 个子代理上限、审批要求、180 秒工具超时均不变；`session_active_list` 仅反映当前进程内存状态（重启后为空，子代理未完成回合已由 `Store::recover()` 结算）。
