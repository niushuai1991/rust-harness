# 计划/目标管理面板规范

- 日期：2026-11-06（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面右侧「计划」只读抽屉：跨会话汇总、`plan_overview` 命令与状态演变历史；不提供计划、目标或待办的直接修改通道。会话计划模式及审批要求见[会话计划模式规范](plan-mode.md)，实现与验证仍在进行。

## 背景与目标

- Todo/Plan/Goal 状态以 `todo.state`/`plan.state`/`goal.state` 事件持久化于 events 表，无独立状态表；事件流是唯一事实来源。
- 会话流卡片只展示当前会话，且随回合折叠；用户没有跨会话纵览所有计划、待办与目标的入口。
- 目标：将桌面端「计划与目标」总览迁入[右侧面板](side-panel.md)的「计划」抽屉，只读汇总所有会话的最新状态，可查看单会话状态历史并跳转对应会话。抽屉不承担计划批准或反馈写入；审批与反馈遵循[会话计划模式规范](plan-mode.md)。

## 范围决策

1. **只读**：面板不提供勾选、编辑或删除。手动写入需要新增 UI 写入通道，且写入不进模型历史，模型无法感知用户侧修改，语义复杂，不做。
2. **全部会话汇总**：不分工作区过滤，展示所有会话的最新状态；按会话分组展示，跨工作区跳转由宿主先切换工作区再定位会话。
3. **历史演变**：单会话历史按需分页读取 `session_timeline_page` 并过滤 `*.state`，最多扫描最近 3 页；不保证覆盖全部历史。
4. 契约演进最小化：仅新增一个派生视图 DTO `PlanStateOverview`，不改事件模型、不建新表。

## 设计

### 契约与存储（`harness-contracts`、`crates/harness-storage/src/lib.rs`）

- `PlanStateOverview`：`{sessionId, sessionTitle, workspaceId, kind, data, updatedAt}`，camelCase，`data` 用既有 `JsonValue` 递归定义。属于派生视图，不落库。
- `Store::latest_states()`：单条 SQL，`GROUP BY session_id,kind` 取 `MAX(seq)` 后 JOIN 回事件行与会话行，固定过滤三种 `*.state` kind（常量内嵌，无外部输入），按会话 `updated_at` 降序、kind 排序。子代理会话同样计入（其状态属于子任务上下文）。

### 桌面命令（`src-tauri/src/commands.rs`）

- 新增 `plan_overview` 命令：直接返回 `store.latest_states()`；注册进 `collect_commands!`。

### 前端（`frontend/`）

- `types.ts` 增加 `PlanStateOverview` 手写镜像。
- `frontend/PlansPanel.tsx` 作为右侧「计划」视图内容，迁移及验证仍在进行：
  - 打开时 `command('plan_overview')` 加载，按会话分组渲染（复用 `.plugin-list`/`.section-label` 等样式）；每会话块显示标题与更新时间。
  - `todo.state` 复用清单渲染（text/content/title + done/completed/status 完成态划线，与 `EventCard` 同规则）；为避免两处维护，把清单渲染从 `EventCard` 提取为共享导出组件 `TodoList`。
  - `plan.state`/`goal.state` 复用 Markdown 渲染规则。
  - 每会话「查看演变历史」按钮：懒加载最多最近 3 页时间线并过滤三种 kind，逐条显示时间、类型与摘要。
  - 「查看会话」按钮回调 `onOpenSession(sessionId, workspaceId)`；同工作区走既有 `openSession`，跨工作区由 App 先切换工作区并在会话列表加载完成后定位目标会话（`pendingJump` ref）。
  - 手动刷新按钮，无自动轮询（状态变化低频，与后台任务面板的 2 秒轮询不同）。
- `App.tsx` 的「计划与目标」入口打开右侧「计划」视图；视图集成与容器行为见[右侧面板规范](side-panel.md)。

## 边界

- 面板为桌面端专属；CLI/headless/ACP 不透传该视图（事件原样可得）。
- 面板只读，无 UI 写入通道；跨工作区跳转依赖既有工作区切换 effect，目标会话缺失时回落列表首个会话。
- 历史仅扫描最近 3 页时间线事件，不等同于完整历史。
- `parent_id` 不参与面板展示（子代理会话以普通会话条目出现，标题区分）。
