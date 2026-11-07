# 自动化任务（周期调度）规范

- 日期：2026-11-07（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面端用户可管理的周期自动化任务：五类周期、可选绑定会话或自动新建会话投递回合；独立管理面板（列表/启停/编辑/删除/立即执行/运行历史）；参考 zcode 自动化取舍。

## 背景与范围

现有调度能力只有模型工具 `schedule_write`（单次、绑定创建会话、missed 不补跑，见 [P2 后台任务规范](p2-background-tasks.md) 与能力矩阵「应用存活期间单次调度」条目），面向模型而非用户。本规范在 `harness-orchestration` 中约定平行的 **Automation（自动化任务）** 机制，由用户在桌面端创建与管理，到期自动向目标会话投递任务内容并执行一个回合。

范围：

- 周期五类：单次（once）、每天（daily）、每周（weekly）、每月（monthly）、每年（yearly）；时间 24 小时制（HH:mm），单次精确到秒。
- 创建任务时可选绑定当前工作区的一个既有会话；未绑定则每次触发自动新建会话。
- 桌面管理面板：列表（周期描述、目标会话、下次执行、运行历史）、启停、编辑（重算下次执行）、删除、立即执行一次。
- 缺日跳过语义：每月 N 日遇短月、每年 2 月 29 日遇平年，跳过该月/该年（与 zcode 日历搜索语义一致）。
- 错过不补跑（沿用既有基线）：应用退出期间到期的任务不补跑；运行期间到期即执行一次并前推（休眠唤醒后最多补当期一次）。

明确不做（后续迭代）：

- 模型工具暴露（模型自建自动化，涉及防递归断言）。
- 结束时间（endAt）/ 最大执行次数（maxRuns）、跨工作区任务、CLI JSON-RPC 暴露。
- 集群/多进程分发（项目为单宿主锁单进程模型）。

## 设计

### 数据与持久化

- settings KV 新键 `orchestration.automations.v1`（与 `orchestration.schedules.v1` 同模式），存 `Vec<Automation>`；与 schedules 共用 `schedules_lock` 串行化读写。
- `Automation`（serde camelCase）：`id`（uuid）、`title`（可空，展示回落 prompt 前缀）、`prompt`、`session_id: Option<String>`（绑定目标会话）、`workspace_id`（创建时工作区，未绑定会话时新建会话归属）、`kind`（`once|daily|weekly|monthly|yearly`）、`at`（once 专用，RFC3339，本地输入转 UTC 存储）、`time`（"HH:MM"，daily/weekly/monthly/yearly 专用）、`weekdays: Option<Vec<u8>>`（weekly 专用，1=周一…7=周日）、`month_day: Option<u8>`（monthly/yearly 专用，1-31）、`month: Option<u8>`（yearly 专用，1-12）、`enabled`、`status`（`active|paused|running|completed|failed|missed|interrupted`）、`next_run_at`（UTC RFC3339）、`last_run_at`、`run_count`、`runs: Vec<AutomationRun>`（内嵌最近 20 条，新记录头插并裁剪）、`created_at`、`updated_at`。
- `AutomationRun`：`id`、`scheduled_at`（本次计划触发时刻）、`triggered_at`、`trigger`（`schedule|manual`）、`status`（`running|succeeded|failed`）、`session_id`、`error`。

### 下次执行时间计算

- 纯函数 `automation_next_run_at(&Automation, from: DateTime<Local>) -> Option<DateTime<Utc>>`：用户时间按本机 Local 时区解释（24 小时制），比较与存储用 UTC RFC3339。
  - once：直接返回 `at`。
  - daily：今天 `time`，已过则明天。
  - weekly：从今天起 8 天内首个「weekdays 命中日 + time」且晚于 from。
  - monthly：逐月搜索（上限 48 个月）首个「存在 month_day 日」的月份的 `time`（短月自然跳过）。
  - yearly：逐月搜索（上限 120 个月）首个「存在 (month, month_day)」的月份（平年无 2/29 自然跳过）。
  - DST 缺失时刻（`LocalResult::None`）按该日无此时刻处理，继续搜索下一天，不特殊处理。
- 校验（创建与更新一致）：`prompt` 非空；once 的 `at` 必须在未来；`time` 必须为合法 HH:MM（0-23:0-59）；weekly 的 weekdays 非空且取值 1-7；monthly/yearly 的 month_day 1-31；yearly 的 month 1-12；任务总数上限 1000（同 schedule）。任一调度字段变更后以当前时刻为基准重算 `next_run_at`。

### 调度与执行

- 复用现有 250ms ticker：`tick()` 处理完 schedules 后串行调用 `tick_automations(recover)`，同一循环天然串行，与 schedules 共用 `schedules_lock`。
- 到期触发（`enabled && status==active && next_run_at <= now`）：先落库再派发——周期任务先把 `next_run_at` 前推到下一触发点（防重启重复），单次任务先置 `running`。
- 执行回合复用 `run_owned`：绑定会话直接投递（会话忙时经既有会话队列锁排队，不抢占）；未绑定先 `store.create_session(workspace_id, title)` 再投递。模型解析：读 `app.default_selection`（modelId+effort）→ `build_model_with`；无默认选择时回落第一个启用的模型；均不可用则该次 run 记 failed，不改 next_run_at 语义（周期任务等待下一触发点）。
- 事件：触发时向目标会话 append `automation.triggered`，完成时 append `automation.finished`（含 status/error）；恢复中断 append `automation.interrupted`。创建/更新/删除/启停为纯 KV 操作，不落事件。
- 恢复语义（启动首轮 recover）：`running` → `interrupted`；once 且已过期 → `missed`；周期且 `next_run_at` 已过 → 重算前推至未来（不补跑、不计 run）。
- 手动立即执行（run_now）：不改 `next_run_at` 与 status，插入 `trigger=manual` 的 run 记录并派发，完成后仅结算该记录与 run_count；暂停状态允许手动执行。

### 桌面命令（src-tauri，JsonPayload 模式同 schedule_list）

- `automation_list(workspace_id)`：按工作区过滤返回。
- `automation_create(workspace_id, title, prompt, session_id, kind, at, time, weekdays, month_day, month)`：校验绑定会话属于该工作区后创建。
- `automation_update(id, ...)`：可改 title/prompt/session_id 与全部调度字段；重算 next_run_at。
- `automation_delete(id)`、`automation_set_enabled(id, enabled)`（恢复启用时若 next_run_at 已过则重算）、`automation_run_now(id)`。
- 命令实现为 `Installed` 上的公开方法（`harness-orchestration`），命令层仅做参数转发与 JsonPayload 包装。

### 前端（React）

- `frontend/types.ts`：手写 `AutomationInfo`/`AutomationRunInfo`（同 `JobInfo` 模式）。
- `frontend/AutomationsPanel.tsx` + `automations.ts`（纯函数模块）+ 配套测试：
  - 列表视图：新建按钮 + 数量上限徽标 + 说明文案；任务卡片（图标、标题、启用徽章、启停开关、周期描述、目标会话、已执行次数、下次执行、运行历史 `<details>`、立即执行/编辑/删除）；删除二段式确认；打开期间 2 秒轮询。
  - 表单视图：标题、任务内容、目标会话（当前工作区会话，默认「每次新建会话」）、执行周期 select、按周期切换的输入（单次 `datetime-local step=1` 默认今天+当前 HH:mm+秒 00；每天 `time`；每周周几多选一~日；每月几号；每年月+日）、缺日提示、内联错误（校验失败保留输入）。
  - `automations.ts` 纯函数：`fmtSchedule`（周期中文描述）、`nextRunAt`（前端镜像计算，仅作预览提示，以后端落库为准）、`defaultOnceValue`（今天+当前 HH:mm+秒 00）。
  - 周期描述格式：`单次 YYYY-MM-DD HH:mm:ss`、`每天 HH:mm`、`每周一、五 HH:mm`、`每月 N 日 HH:mm`、`每年 M 月 N 日 HH:mm`。
- `App.tsx`：`Panel` 联合类型加 `'automations'`，侧栏底部按钮（Timer 图标，位于「能力预设」与「插件与权限」之间），Modal 渲染（传当前工作区 sessions 供会话选择）。
- `style.css`：`.auto-*` 系列样式，沿用现有固定色值与 `.settings-form`/`.plugin-list` 模式，无新增依赖。

## 边界

- 时区按本机 Local；DST 切换时刻的 HH:mm 取 chrono 常规行为，缺失时刻按该日无此时刻跳过。
- 运行历史内嵌 KV（每任务 ≤20 条）而非独立表：任务量小（上限 1000）、免 schema 迁移；超量裁剪最旧记录。
- 前端预览时间与后端计算为两份实现（镜像逻辑），预览仅提示，落库以后端为准；两者偏差不阻塞主流程。
- 绑定会话在执行时可能已被删除：投递前校验会话存在，缺失则该次 run 记 failed（不自动改绑）。
- 手动执行与计划触发可能在同一会话上并发排队，由既有会话队列锁串行，不合并。
