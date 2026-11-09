# 会话列表「最近 / 项目」视图规范（参考 zcode）

- 日期：2026-11-08（计划拟制）；2026-11-09 修订（侧栏会话入口重整、标题生成与重命名）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面侧栏任务列表的两种显示模式（按最近 / 按项目）、项目置顶持久化、标题搜索过滤、跨工作区会话跳转、首问标题生成与会话重命名；不改变公开会话字段与跨端同步语义。

## 背景与痛点

改造前侧栏会话列表只显示当前工作区的平铺会话（后端固定 `updated_at` 倒序）：

- 无分组、无时间信息，会话数量增多后难以定位；
- 无法总览其他项目的会话，查看其他项目必须先切换工作区；
- 后端 `session_search`（FTS 全文检索）能力已就绪，但前端没有暴露任何搜索入口。

参考 zcode 的会话历史（按最近 / 按项目两种视图，置顶项目常驻列表头部）做对齐。

## 设计

### 视图与排序

1. **任务列表标题行**：侧栏分区标题为「任务列表」，右侧为圆角方块 switch（内嵌「最近 / 项目」两词，白色滑块盖在哪侧哪侧以主色显示）；标题行不放新建按钮。品牌区下方、搜索框上方为常显「新建聊天」按钮：点击进入**新会话草稿态**（不立即创建会话），落点默认当前激活项目，composer 上方出现项目 chip 供切换或新建项目，发送首条消息时才真正创建会话（参考 zcode 草稿态 `ChatEmptyWorkspacePreviewMenu` 交互；2026-11-09 修订）。项目组头「+」仍直接创建（组头已显式指定落点项目）。
2. **按最近**（默认）：跨工作区全量会话按 `updated_at` 倒序平铺，按本地日历日分为「今天 / 昨天 / 近 7 天 / 更早」四个粘性分组；每项两行：标题 + 子任务徽章 + 相对时间，第二行显示所属项目名，置顶项目的会话带图钉标记（不改变时间序）。
3. **按项目**：有会话的工作区成组，置顶组排最前，其余按组内最新会话倒序；组头含折叠箭头、项目名、会话数、新建聊天按钮（常显）与置顶按钮（hover 浮现，置顶后常显）；组头新建按钮对当前项目直接走新建流程，对其他项目先为该项目创建会话再经 pendingJump 切换工作区并定位到新会话；组内单行条目按 `updated_at` 倒序。
4. 相对时间标签：今天按分钟/小时，昨天为「昨天 HH:mm」，近 7 天按星期，更早为「M月D日」（跨年含年份）。时间分组与标签是 `frontend/sessionList.ts` 纯函数，注入 `now` 便于测试。

### 置顶持久化

- 置顶集合存 SQLite `settings` KV 键 `workspace.pinned.v1`（JSON 字符串数组）；`Store::pinned_workspaces` / `Store::set_workspace_pinned` 读写并去重。
- `WorkspaceInfo` 新增 `pinned: bool`（serde default；工作区 `record.json` 本身不承载置顶语义，由桌面命令层从 KV 派生注入）；`workspace_list` 返回时注入置顶状态。
- 新增 `workspace_set_pinned { workspaceId, pinned }` 命令：先校验工作区存在再写 KV，返回更新后的 `WorkspaceInfo`。
- 前端置顶为乐观更新，失败回滚并显示错误。

### 数据加载与跳转

- `session_list` 新增可选参数 `all_workspaces`：true 时返回全部工作区会话（仍按 `updated_at` 倒序，附带运行时父子关系推导）；缺省行为不变（按工作区过滤），CLI / JSON-RPC 契约不受影响。
- 桌面端拉取全量后本地过滤出当前工作区列表（既有 `sessions` 语义不变），全量存 `allSessions` 供侧栏渲染；新建会话同步插入两个列表，且即便其所属工作区已不再是激活工作区（创建迟到完成）也并入 `allSessions`（侧栏可见、可点击返回），但不改变当前激活会话与发送目标。
- 侧栏点击非当前工作区的会话：先切换工作区，再经既有 `pendingJump` 机制跳转到该会话；同工作区走既有 `openSession`。
- 侧栏不设工作区切换下拉框（2026-11-09 移除）：切换项目的入口是点击侧栏会话（跨工作区经 pendingJump）与项目组头新建按钮。
- 视图模式与项目折叠状态存 localStorage（`sessionView` / `sessionCollapsed`），跨重启保留。

### 搜索过滤

- 侧栏搜索框对会话标题做本地大小写不敏感过滤，两种视图共用；无匹配时显示空态文案。

### 标题生成与重命名

- 桌面端首条用户消息发送时，使用当前模型与推理档位异步生成简短标题，并与首轮回复并行；独立标题请求不写入会话事件或上下文。标题生成不可用、超时或返回空内容时，用首条消息的首个非空行作标题；纯附件消息用首个附件文件名作标题。
- 新建占位标题的状态存 SQLite `sessions.title_state`：`default` 可自动生成，`generated` 已自动命名，`manual` 已手动命名。自动写入仅在状态仍为 `default` 时成功；重命名立即设为 `manual`，因此用户标题优先于并发中的生成任务。
- SQLite schema v3 从 v2 迁移：仅将标题仍为「新会话」且没有 `user.message` 事件的旧记录标记为 `default`，其他旧记录设为 `manual`；公开 `SessionInfo` 结构保持不变。
- 最近与项目两种视图中的会话行均支持右键菜单「重命名」；对话框预填现标题。`session_rename { sessionId, title }` 去除首尾空白，空白标题无效，标题沿用 512 字节上限，成功后返回更新的 `SessionInfo`。
- `session_title_generate { sessionId, text, modelId, effort, attachmentNames }` 返回最终 `SessionInfo`；标题命令失败不影响消息发送。标题生成与重命名只影响桌面端界面，不扩展 CLI / JSON-RPC / ACP 标题行为。

## 非目标

- 不做会话移动；不改变公开会话字段与跨端同步语义。
- 不接入 FTS 全文检索（`session_search` 留待后续叠加）。
- CLI / JSON-RPC / ACP 不暴露视图与置顶概念（置顶是桌面端用户偏好，不进入工作区记录）。

## 验证

- `harness-storage`：置顶集合读写、重开持久化与去重（`tests/workspace_pins.rs`）。
- `harness-storage`：v2→v3 标题状态迁移、占位标题识别、手动重命名校验与自动标题条件更新（`tests/session_titles.rs`）。
- `sessionList` 纯函数：时间分组、相对时间标签、标题过滤、项目分组与置顶排序（`frontend/sessionList.test.ts`）。
- `App`：最近 / 项目切换、置顶命令调用与组间排序、置顶失败回滚、搜索过滤与空态、侧栏会话入口、首问标题并行生成、右键重命名与菜单关闭、手动标题优先（`frontend/App.test.tsx`）。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。
