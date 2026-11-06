# P2 后台任务桌面可视化计划

计划拟制日期：2026-11-06。

## 背景与范围

P2 阶段验收项「后台任务」的后端能力已由 `harness-orchestration` 实现（后台 Shell 的 `background_run/status/cancel`、子代理的 `agent_fork/send/wait`、单次调度的 `schedule_write/list/delete`），并有 7 项单测覆盖。但桌面端存在可见性缺口：

- `job.started/finished/cancel_requested`、`schedule.*`、`subagent.*` 事件在消息流中不渲染，模型调用后台任务时用户在界面上看不到任何反馈。
- 没有后台任务管理入口，无法查看运行中的后台 Shell 任务与定时任务，也无法人工取消或删除。

本计划补齐该闭环：消息流事件卡片 + 会话级后台任务面板 + 宿主查询/操作命令。不改变后台任务的后端语义（审批、工作区锁、事件持久化、重启不重放）。

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
- 事件卡片只读呈现，不承担操作；操作集中在面板。
- 跨会话任务中心、完成通知、PTY 不在本次范围。

## 验证

- Rust：`harness-orchestration` 新增公共 API 测试（会话作用域、取消归属校验、定时删除事件）；`cargo fmt --check`、`clippy -D warnings`、`test --all-features`、`build --all-features`。
- 前端：事件卡片渲染测试、面板加载/取消/删除/作用域错误测试；`npm test`、`npm run build`。
- 绑定：运行 `export_bindings` 测试后 `bindings.ts` 与命令一致。
