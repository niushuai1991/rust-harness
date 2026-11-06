# 计划/目标管理面板计划

拟制日期：2026-11-06。

状态：计划拟制（2026-11-06），实施记录见文末。

依据：P2 阶段验收项「计划/目标」中，模型工具（`todo_write`/`plan_write`/`goal_write`，免审批，写入 `*.state` 事件）与会话流卡片已随子代理可视化专项交付；能力矩阵与子代理可视化计划均记载「独立管理面板未实现」。经用户确认两个范围决策：**只读查看**（不新增 UI 写入通道）、**全部会话汇总**（跨会话才是独立面板的增量价值）。据此拟制本计划。

## 背景与目标

- Todo/Plan/Goal 状态以 `todo.state`/`plan.state`/`goal.state` 事件持久化于 events 表，无独立状态表；事件流是唯一事实来源。
- 会话流卡片只展示当前会话，且随回合折叠；用户没有跨会话纵览所有计划、待办与目标的入口。
- 目标：新增桌面端「计划与目标」管理面板，只读汇总所有会话的最新状态，可查看单会话的状态演变历史并跳转对应会话。

## 范围决策

1. **只读**：面板不提供勾选、编辑或删除。手动写入需要新增 UI 写入通道，且写入不进模型历史，模型无法感知用户侧修改，语义复杂，不做。
2. **全部会话汇总**：不分工作区过滤，展示所有会话的最新状态；每条附会话标题与所属工作区，跨工作区跳转由宿主先切换工作区再定位会话。
3. **历史演变**：单会话的 `*.state` 事件历史经既有 `session_events` 命令懒加载过滤，不新增后端命令。
4. 契约演进最小化：仅新增一个派生视图 DTO `PlanStateOverview`，不改事件模型、不建新表。

## 设计

### 契约与存储（`harness-contracts`、`crates/harness-storage/src/lib.rs`）

- `PlanStateOverview`：`{sessionId, sessionTitle, workspaceId, kind, data, updatedAt}`，camelCase，`data` 用既有 `JsonValue` 递归定义。属于派生视图，不落库。
- `Store::latest_states()`：单条 SQL，`GROUP BY session_id,kind` 取 `MAX(seq)` 后 JOIN 回事件行与会话行，固定过滤三种 `*.state` kind（常量内嵌，无外部输入），按会话 `updated_at` 降序、kind 排序。子代理会话同样计入（其状态属于子任务上下文）。

### 桌面命令（`src-tauri/src/commands.rs`）

- 新增 `plan_overview` 命令：直接返回 `store.latest_states()`；注册进 `collect_commands!`。

### 前端（`frontend/`）

- `types.ts` 增加 `PlanStateOverview` 手写镜像。
- 新建 `frontend/PlansPanel.tsx`（`Modal wide`）：
  - 打开时 `command('plan_overview')` 加载，按会话分组渲染（复用 `.plugin-list`/`.section-label` 等样式）；每会话块显示标题、工作区与更新时间。
  - `todo.state` 复用清单渲染（text/content/title + done/completed/status 完成态划线，与 `EventCard` 同规则）；为避免两处维护，把清单渲染从 `EventCard` 提取为共享导出组件 `TodoList`。
  - `plan.state`/`goal.state` 复用 Markdown 渲染规则。
  - 每会话「查看演变历史」按钮：懒加载 `session_events` 过滤三种 kind，逐条显示时间、类型与摘要，最新一条标记「当前」。
  - 「查看会话」按钮回调 `onOpenSession(sessionId, workspaceId)`；同工作区走既有 `openSession`，跨工作区由 App 先切换工作区并在会话列表加载完成后定位目标会话（`pendingJump` ref）。
  - 手动刷新按钮，无自动轮询（状态变化低频，与后台任务面板的 2 秒轮询不同）。
- `App.tsx`：`Panel` 联合增加 `'plans'`；侧边栏底部新增「计划与目标」入口按钮；条件渲染挂载面板。

## 测试（TDD）

- `crates/harness-storage/tests/plan_states.rs`（新文件，先红后绿）：
  - 同会话同 kind 多次写入只取最新一条；不同 kind 各取最新；
  - 无状态事件的会话不出现；排序按会话 `updated_at` 降序；
  - 附带 `session_title`/`workspace_id` 正确。
- 前端 `frontend/PlansPanel.test.tsx`（新文件）：分组渲染、todo 完成态、plan/goal Markdown、跳转回调参数、历史懒加载与空态。
- `frontend/App.test.tsx` 增加 1 项：侧边栏入口打开面板并展示汇总。

## 边界

- 面板为桌面端专属；CLI/headless/ACP 不透传该视图（事件原样可得）。
- 历史懒加载受既有 `session_events` 全量返回约束，超大会话一次性拉取；与重启回放路径一致，可接受。
- `parent_id` 不参与面板展示（子代理会话以普通会话条目出现，标题区分）。

## 实施记录（2026-11-06）

- TDD：`crates/harness-storage/tests/plan_states.rs` 2 项先红（`latest_states` 方法缺失编译失败）后绿；前端 `PlansPanel.test.tsx` 5 项与 `App.test.tsx` 入口用例 1 项先红后绿（测试自身修正三处：todo 项文本含 `✓ ` 前缀须用 querySelector 断言；当前会话按钮文案为「当前会话」；关闭面板是宿主 `openFromPanel` 的责任，组件只回调 `onOpenSession`）。
- 全套验证：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`（125 项）、`cargo build --all-features`、`npm ci` + `npm test`（51 项）、`npm run build` 通过；`frontend/bindings.ts` 经 `export_bindings` 测试重新导出（新增 `plan_overview` 与 `PlanStateOverview`）。
- 顺手修复：`harness-orchestration::session_schedules` 未使用变量告警（PR #14 合并遗留），`runtime()?` 调用保留 SHUTDOWN 检查语义。
- 边界确认：面板只读，无 UI 写入通道；跨工作区跳转依赖既有工作区切换 effect，目标会话缺失时回落列表首个会话；历史懒加载复用既有 `session_events` 命令；`TodoList` 从 `EventCard` 提取为共享导出组件，会话流卡片渲染不变。
