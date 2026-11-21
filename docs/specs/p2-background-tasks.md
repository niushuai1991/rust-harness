# P2 后台任务桌面可视化规范

- 日期：2026-11-06（计划拟制）；修订：2026-11-22（后台任务扩展到全部执行位置、新增 shell 前台到限自动转后台与 composer 运行中任务浮层）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：消息流事件卡片 + 会话级后台任务面板 + 宿主查询/操作命令 + 输入框终端图标浮层；审批、工作区锁、事件持久化、重启不重放语义不变。

## 修订记录

- 2026-11-22：`background_run` 从「仅托管副本」扩展到会话执行位置（直接模式/worktree/副本/scratch），按会话执行位置镜像前台 shell 执行方式（详见[直接执行模式](direct-execution.md)）；shell 前台执行到达时限（KV `app.tool_timeout_secs`，缺省 120 秒）自动转为受管后台任务继续运行，不取消、不重跑；内存任务注册表从 harness-orchestration 上移到 `Runtime::jobs`，前台转后台与 `background_run` 共用；输入框下方新增终端图标浮层展示运行中任务与已运行时长。

## 背景与范围

P2 阶段验收项「后台任务」的后端能力由 `harness-orchestration` 实现（后台 Shell 的 `background_run/status/cancel`、子代理的 `agent_fork/send/wait`、单次调度的 `schedule_write/list/delete`）。桌面端存在可见性缺口：

- `job.started/finished/cancel_requested`、`schedule.*`、`subagent.*` 事件在消息流中不渲染，模型调用后台任务时用户在界面上看不到任何反馈。
- 没有后台任务管理入口，无法查看运行中的后台 Shell 任务与定时任务，也无法人工取消或删除。

本规范约定补齐该闭环的界面与宿主接口设计。

## 设计

### 事件渲染（frontend/App.tsx `EventCard`）

| 事件 | 渲染 |
|---|---|
| `job.started` | 后台任务卡片：命令摘要 + 「运行中」状态，展开显示完整命令 |
| `job.finished` | 同卡片更新为终态（已完成/失败/已取消/已中断），展开显示退出码、stdout/stderr 或错误 |
| `job.cancel_requested` | 状态行「已请求取消」 |
| `schedule.created` | 定时任务卡片：时间、任务、模型 |
| `schedule.deleted` | 状态行 |
| `schedule.running/finished/failed/missed/interrupted` | 状态行（完整状态在面板中查看） |
| `subagent.created`（非 historyOnly） | 子代理卡片：来源 |
| `subagent.message_sent` | 子代理投递卡片：任务内容 |

### 宿主公共 API（harness-orchestration）

在 `Installed` 上新增人机接口（与模型工具同源实现，不复刻逻辑）：

- `session_jobs(session_id) -> Vec<JobInfo>`：扫描会话事件推导每个任务的最新状态（started/finished/cancel_requested 合成），按启动顺序返回；`JobInfo{id,command,status,cancelRequested,output,error,createdAt}`。
- `cancel_job(session_id, job_id)`：校验会话归属后取消令牌并追加 `job.cancel_requested`；非本会话任务报 `JOB_SCOPE`。
- `session_schedules(session_id) -> Vec<Schedule>`：按会话过滤定时任务；`Schedule` 公开为 serde camelCase DTO。
- `delete_schedule(session_id, id)`：校验会话归属后删除并追加 `schedule.deleted`；工具 `schedule_delete` 复用同一实现。

### Tauri 命令（src-tauri）

`job_list`、`job_cancel`、`schedule_list`、`schedule_delete`，均带 `sessionId`；经 `tauri-specta` 重新导出 `frontend/bindings.ts`。

### 后台任务面板（frontend/BackgroundTasksPanel.tsx）

- 顶栏新增「后台任务」按钮，徽标数字为当前会话未终结的后台 Shell 任务数（由已加载事件推导，不新增轮询）。
- 面板为会话级 Modal，两个分组：
  - 「后台任务」：命令、状态徽标、创建时间；运行中任务提供「取消」。
  - 「定时任务」：执行时间、任务、模型、状态；`pending` 任务提供「删除」。
- 打开期间每 2 秒轮询 `job_list` 与 `schedule_list`（与审批轮询同模式）。空闲期完成的任务依赖面板轮询与刷新，不在消息流额外推送。

## 边界

- 面板与命令均为人工接口，不向模型工具暴露新能力；工具审批策略不变。
- 事件卡片只读呈现，不承担操作；操作集中在面板与输入框浮层。
- 跨会话任务中心、完成通知、PTY 不在范围。

## shell 前台到限自动转后台（2026-11-22 修订）

- shell 工具前台执行到达时限（KV `app.tool_timeout_secs`，缺省 120 秒、允许 5–3600，桌面「设置 → 性能」暴露）时**不取消**：已在执行的工具体原样移入独立任务继续运行，无重跑副作用。
- 转后台沿用本调用前已通过的命令审批，不发起新审批；沙箱提权重跑若已裁决，后台以同款方式继续执行。
- 转后台时登记 `Runtime::jobs`（与 `background_run` 共用的内存注册表），落 `job.started` 事件；工具体结束后还原 shell 结果（`ProcessOutput` JSON）落 `job.finished`，事件形状与 `background_run` 完全一致，`background_status` 查询与桌面面板零改动可用。
- 工具向模型返回任务 id 与 `background_status`/`background_cancel` 用法提示；回合结束后任务继续存续，可经注册表令牌取消，宿主退出统一取消，重启后未终结任务由恢复逻辑补记「已中断」且不重放副作用。
- 非 shell 工具到达时限维持「取消 + TOOL_TIMEOUT（结果未知）」现状。

## 输入框终端图标浮层（frontend/ComposerJobsPanel.tsx）

- 输入框下方、子代理图标左侧新增终端图标；仅已建会话渲染，徽标数字与顶栏「后台任务」按钮同源（已加载事件推导的未终结任务数）。
- 点击弹出浮层卡片：仅列本会话**运行中**的后台 Shell 任务（含前台转后台与模型主动发起），每条显示命令摘要、已运行时长（展开期间秒级刷新，自启动时刻起算）与取消按钮；`cancelRequested` 的任务显示「已请求取消」不再提供取消。
- 点开关按钮、点外部或按 Escape 收起；数据与顶栏面板同源（`job_list` 每 2 秒轮询），顶栏面板保持不变。
