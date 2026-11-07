# CLI 命令行 Agent 调用规范

- 日期：2026-11-05（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：`harness` 命令行 run/chat/diff/apply/models/sessions 能力，参考 codex CLI 设计（本地仓库 `D:\code\codex`）。

## 背景与目标

`harness` 二进制（`crates/harness-cli`）此前只提供 JSON-RPC 接口（`rpc` stdio 长连接、`call` 单次调用），在命令行完成一次 Agent 任务需要 SDK 或手工构造 JSON-RPC 报文。本规范约定直接的命令行能力：

- `run`：一次性执行任务，流式输出过程与结果后退出（对标 `codex exec`）。
- `chat`：终端交互式循环对话（简化版交互模式）。
- `diff` / `apply`：审阅隔离工作副本的具体改动并同步回原项目，补全项目核心工作流的命令行闭环。
- `sessions` / `models`：会话查看与模型管理。

## 设计原则（借鉴 codex CLI）

- 宿主薄、core 厚：CLI 子命令只是 `harness-runtime` 的薄宿主，内部复用现有 `dispatch()` 方法层（相当于 codex 的内嵌 app-server），不在 CLI 重复实现业务逻辑；个别 CLI 专属设置（如 `cli.default_model`）经 `Store::get/set` 直接读写，并在本规范中显式列出，不虚称"全部走 dispatch"。
- 单一事件流、多种渲染器：所有输出都是对 `Runtime::subscribe()` 的 `LiveEvent` 流的消费策略差异，参考 codex `EventProcessor` trait 抽象出人类可读与 JSONL 两种渲染器。
- 协议与宿主解耦的既有格局不变：桌面端继续走 Tauri IPC，CLI 与 SDK 继续共用 JSON-RPC 方法语义。
- CLI 需要而 runtime 缺失的能力，以最小幅度的 runtime 增强补齐（见下节），并保持既有消费者语义不变。

## runtime 增强（本规范包含的最小改动）

CLI 的作用域隔离无法纯客户端实现：runtime 在写入 `inbox.queued`（含私有 `message_id`）之后才等待会话队列锁，获得锁后才另行生成 `run_id` 并写入 `turn.start`，而 `session.send` 只返回 `null`，调用方无法证明"下一条 `turn.start` 属于自己发送的消息"。为此新增以下 runtime 改动（桌面端与 SDK 消费者语义均不变）：

1. **拆分入队与执行**：将 `run_session_with_attachments` 拆为
   - `Runtime::enqueue_message(session, text, attachments) -> AppResult<String>`：输入校验、附件校验与 `inbox.queued` 落库，立即返回 `message_id`；
   - `Runtime::execute_message(session, message_id, provider) -> AppResult<()>`：获取会话队列锁 → 取消检查 → `turn.start` → `inbox.claimed` → `user.message` → `steps()` → `turn.end` → `Status` 广播。
   - 原函数保留为两者的顺序组合包装，签名与"调用即等待回合结束"的语义不变；`dispatch` 的 `session.send` 与 orchestration 后台任务继续走包装函数。
2. **回合事件携带 `messageId`**：`turn.start`/`turn.end` 载荷在既有 `runId` 基础上增加 `messageId` 键。旧消费者忽略未知键，向后兼容。
3. **排队消息取消**：新增 `Runtime::cancel_message(session, message_id)` 登记"待取消消息"；`execute_message` 获得队列锁后检查命中，命中则落 `inbox.cancelled {"id":messageId}` 并以 `CANCELLED` 结束——不产生 `turn.start`/`turn.end`、不调用模型，避免取消误伤排队在自己前面的其他回合。

以上改动均在 `crates/harness-runtime/src/agent.rs`，须以 `harness-runtime` 测试兜底并回归桌面端命令与 SDK。

## 运行作用域

事实约束：`Runtime::subscribe()` 是全部会话共享的 broadcast 通道（容量 4096）；`orchestration::install()` 启动常驻调度器 tick，可能在任意会话产生事件、排队回合与审批；`agent_spawn` 会在子会话中启动独立回合；同会话回合经队列锁串行，但同一会话可能已有排队回合。

### 回合归属与括号

1. CLI 经 `enqueue_message` 直接取得自己的 `message_id`，不依赖事件内容猜测。
2. 仅认领 `turn.start` 载荷 `messageId` 等于自己的事件为括号起点；`turn.end` 载荷 `messageId` 相同才判本轮结束。排队回合、定时任务触发的回合不可能被误认领。
3. 事件过滤按 `session_id`（`Delta`、`Status` 自带 `sessionId`；`Durable` 事件经 `SessionEvent.session_id` 判定）。

### 子代理作用域

- 作用域集合 = 父会话 ∪ 运行期间发现的递归后代：任意作用域内会话发出的 `subagent.created {"sessionId"}` 事件将子会话入集（子会话自身的 `subagent.created` 同样生效）；父括号关闭后以 `Runtime::children_of` 兜底核对。
- 审批与渲染作用于**作用域集合内所有会话**：子会话回合有自己的 `runId`，不参与父括号；`auto`/`ask` 均处理集合内全部 `approval.request`，否则子代理首次写文件或运行 shell 即等待 600 秒超时。
- 完成判定：父括号关闭后进入**子代理收尾**阶段，继续消费事件与处理审批，直到作用域内无 `Runtime::is_active` 会话；收尾上限 600 秒（与审批等待一致），期间 Ctrl+C 对作用域内全部活动会话执行 `cancel` 并以 130 退出。
- 子会话的输出在 human 模式加 `[子代理 <id 前缀>]` 前缀。

### 取消

- 括号未开启（消息仍在排队）→ `cancel_message(session, messageId)`，等待 `inbox.cancelled` 后退出码 130。
- 括号已开启 → `cancel(sessionId)` 取消自己的活动回合，等待同 `messageId` 的 `turn.end`（≤10 秒）后退出码 130。
- 二次 Ctrl+C 立即以 130 退出。

## 审批策略

平台报告明确当前沙箱边界："WRITE_RESTRICTED+Low IL 提供真实系统写入边界，**不隔离读取和网络**"。隔离工作副本与 apply 人工审阅是"改动不进原项目"的边界，不是工具执行副作用的边界：`shell` 可读取系统文件并访问网络，`auto` 还会放行后台任务、定时任务与子代理。据此审批策略如下：

| 模式 | 行为 | 副作用边界 |
|---|---|---|
| `ask`（默认，stdin 为 TTY） | 终端展示会话标识（子代理显式标注）、工具名与参数，等待 y/n；600 秒无响应视为拒绝 | 与桌面端审批同边界 |
| `auto`（须显式选择） | 自动允许**作用域集合内所有会话**的审批请求（含子代理会话） | `shell` 可读系统文件、访问网络；可拉起后台任务、定时任务、子代理；改动仍只落副本，进原项目需 `apply` |
| `never` | 一律拒绝作用域内需审批工具 | 仍可执行免审批白名单：`read`/`read_artifact`/`list_files`/`search`/`agent_status` 只读；`todo_write`/`plan_write`/`goal_write` 会持久写入会话事件库（非文件系统、不触碰副本与原项目） |

默认规则：`--approval` 缺省时，stdin 为 TTY 取 `ask`；stdin 非 TTY 时启动报错，要求显式传 `auto|ask|never`。`ask` 在 stdin 非 TTY 时一律启动报错。

## stdin 与审批行为矩阵

| 提示词来源 | stdin 类型 | `ask` | `auto`/`never` |
|---|---|---|---|
| 命令行 PROMPT | TTY | 正常执行；审批从 stdin 读 y/n | 正常执行 |
| 命令行 PROMPT | 管道/重定向 | 启动报错（stdin 非交互，无法确认） | 正常执行 |
| `--prompt -` | 任意 | 启动报错（同上） | 正常执行；提示词读 stdin 至 EOF |
| 缺省 PROMPT | TTY | 启动报错（对齐 codex exec：不挂起等待） | 同左 |
| 缺省 PROMPT | 管道 | — | 读 stdin 至 EOF 作为提示词；输入为空报错 |

`chat` 的 stdin 消费约定：REPL 行读取与审批 y/n 读取是**同一 stdin 的串行消费者**——回合进行中 REPL 不读取输入，审批完成后才回到提示符；EOF 或 `/quit` 退出。`chat` 启动时校验 stdin 为 TTY，否则报错。

## 工作区与会话初始化规则

- `--session <id>` 与 `--workspace <id>` 互斥，同时给出即报错。
- `--session <id>`：以该会话绑定的 `workspaceId` 为准；若同时给 `--cwd`，其 canonicalize 后不等于该工作区 `source_path` 即报错。
- 未给 `--session` 时：canonicalize 目标目录（`--cwd` 或进程当前目录），在 `workspace.list()` 中按 `source_path` 匹配：
  - 无匹配 → `workspace.create`；
  - 多个匹配 → 取 `created_at` 最新；`--workspace <id>` 显式指定优先；
  - 副本存在未应用改动（`workspace.diff` 非空）→ 拒绝启动，提示先经 `harness diff`/`apply` 处理，或用 `--workspace <id>`/`--session <id>` 显式复用（显式复用时打印警告，避免旧改动污染新任务）。
- `chat` 初始化：cwd 规则同 run；模型 = `-m` > settings `cli.default_model` > 报错并列出可用模型；会话默认新建，`--session` 续聊；审批默认 `ask`（chat 必然 TTY），`/approval` 运行时切换；`/model` 切换只影响后续回合。

## 其余设计决策

1. **REPL 形态**：简单 readline 循环加流式纯文本输出，不引入 ratatui 全屏 TUI。（体验升级已由[输入框斜杠快捷指令规范](slash-commands-spec.md)落地：crossterm 行编辑与 `/` 补全弹窗，仍不做全屏 TUI。）
2. **默认模型**：优先级为 `-m/--model` > settings 键 `cli.default_model`（经 `Store::get/set` 读写）> 报错并列出可用模型。`models default <id>` 设置前经 `models.list_models()` 校验存在，不存在拒绝保存；`models default`（无参）查看当前值；`models default --clear` 清除（写 `Null` 视为未设置）。
3. **参数解析**：`clap 4`（derive）；`rpc` 与 `call` 行为保持不变。
4. **数据目录**：维持现状（`--data-dir`、环境变量 `RUST_HARNESS_DATA_DIR`、默认 `%LOCALAPPDATA%\RustHarness`），`host.lock` 独占与桌面端互斥的关系不变。

## 命令规格

```text
harness run [PROMPT]     # 别名 e；一次性执行，流式输出，结束退出
  -C/--cwd <path>        # 目标项目目录，默认当前目录；工作区解析规则见上节
  -m/--model <id>        # 模型选择，见默认模型决策
  --workspace <id>       # 显式指定工作区，与 --session 互斥
  --session <id>         # 续已有会话；缺省新建（标题取提示词前缀）
  --prompt -             # 从 stdin 读提示词至 EOF
  --json                 # JSONL 事件流输出到 stdout，契约见"输出与取消契约"
  --approval auto|ask|never
  --color auto|always|never
  PROMPT 缺省时按 stdin 矩阵处理
  退出码：同 messageId 的 turn.end 为 completed 返回 0；error 或发送失败返回 1；取消（Ctrl+C / cancel_message）返回 130
  结束时副本存在改动则在 stderr 提示 diff/apply 命令

harness chat             # readline 循环；斜杠命令见[输入框斜杠快捷指令规范](slash-commands-spec.md)

harness sessions [list|show <id>]
  list 走 dispatch session.list；show 打印会话信息与尾部事件摘要（store.session + session.events）

harness diff <workspaceId> [--color ...]
  渲染契约见"diff 审阅与同步契约"

harness apply <workspaceId> [--all]
  交互逐文件 y/n/a/q（a=全部剩余，q=中止）；--all 跳过确认；选择集仅含非冲突文件，契约见下节

harness models [list|test <id>|default [<id>|--clear]]
（保留 rpc / call）
```

## diff 审阅与同步契约

`workspace.changes()` 的判定事实：`kind` 由基线与副本快照推导——before 缺失为 `added`、after 缺失为 `deleted`、两侧均在为 `modified`；`before_text/after_text` 是 UTF-8 解码结果，解码失败为 `None`；`conflict` 表示原项目侧内容偏离基线。`apply_inner` 对含 `conflict=true` 的选择**整体返回 `CONFLICT`**，且基线读取无字节上限。据此：

- **按 `kind` 解释文本缺失**：`added` → before 视为空文本（全部渲染为新增行）；`deleted` → after 视为空文本（全部渲染为删除行）；`modified` 且任一侧缺失 → 视为二进制/无法解码，不渲染正文，显示 before/after 哈希提示。
- **渲染前置字节阈值**：任一侧文本超过 1 MiB 时不生成 unified diff（阈值必须在调用 `similar` 前生效，不能靠生成后截断控制内存与计算量），显示两侧字节统计与哈希。
- **显示层截断**：单文件 diff 输出超过 2000 行截断并提示。
- **冲突文件不可 apply**：`conflict=true` 的文件在 diff 输出中显式标注 `CONFLICT` 并着色，注明"暂不可 apply，需先处理原项目侧改动"；交互确认与 `--all`/`a` 的选择集**自动排除冲突文件**（排除项在确认清单中列明），提交 `apply_reviewed` 的快照保持哈希一致；其内置哈希重校验失败（`STALE_DIFF`）按原错误整体呈现，不做静默重试。
- **颜色**：`--color auto|always|never`，默认 auto（非 TTY 不着色）。
- **apply 交互**：逐文件 y/n/a/q（a=全部剩余非冲突文件，q=中止）；`--all` 跳过确认。

## 模块结构

```text
crates/harness-runtime/src/agent.rs
└── （本规范的最小 runtime 增强：enqueue/execute 拆分、turn 事件含 messageId、cancel_message）

crates/harness-cli/
├── src/main.rs         # clap Parser + 子命令分发；保留 --data-dir 解析逻辑
├── src/lib.rs          # JSON-RPC dispatch/serve
├── src/console.rs      # 渲染器（human：正文 stdout、工具/审批/推理 stderr；json：逐行 JSON）
├── src/approval.rs     # auto|ask|never 策略，作用域集合内审批处理
├── src/scope.rs        # messageId 认领、子代理作用域集合、Lagged 降级
├── src/diffview.rs     # unified diff 渲染（re-export harness-platform 共享实现，见[右侧面板规范](side-panel-spec.md)）
├── src/run.rs          # 一次性执行（初始化规则 → enqueue → 事件消费 → 同 messageId turn.end）
├── src/chat.rs         # REPL（复用 run 的单轮执行内核）
├── src/manage.rs       # sessions/diff/apply/models
└── tests/run.rs        # 集成测试
```

## 输出与取消契约

- 人类模式：`Delta` 正文（`reasoning=false`）→ stdout；工具调用摘要、审批提示、错误、reasoning → stderr。`assistant.message` 持久事件不重复打印，以 `request.snapshot.id ↔ Delta.requestId` 关联判定：`request.snapshot`（id=R）之后收到过 ≥1 条 `Delta(R, reasoning=false)` 才抑制其后相邻的 `assistant.message`；仅收到 reasoning Delta、或自上一次判定以来发生过 Lagged 时，强制打印完整正文。
- JSON 模式（`--json`）：stdout 仅输出 JSONL 且每行必须可独立解析：
  - `{"type":"delta","sessionId","requestId","index","text","reasoning"}`
  - `{"type":"event","event":<SessionEvent>}`
  - `{"type":"event_gap","sessionId","fromSeq","toSeq"}`（Lagged 时输出；缺失的 `Delta`/`Status` 不可回补，此行即降级声明）
  - 终行 `{"type":"result","messageId","runId","sessionId","workspaceId","status":"completed|error|cancelled","exitCode":0|1|130,"finalMessage":<string>,"changedFiles":<n>}`
  - 人类可读摘要（含 diff 提示）一律 stderr，保证 stdout 可安全管道消费。
- Lagged 降级：broadcast `recv()` 返回 `Lagged` 时，以 `store.events(session, after=最后已见 seq)` 回补 durable 事件，stderr（human）或 `event_gap`（json）声明缺口；该回合内后续 `assistant.message` 强制打印全文以重建正文。
- 取消：见"运行作用域 → 取消"。

## 风险与约束

- 本规范包含最小 runtime 增强（回合流程拆分、事件新增键、排队取消），触及 `harness-runtime` 核心路径，须以 `harness-runtime` 测试与桌面端回归兜底。
- `auto` 为显式选项且默认关闭（默认 `ask`）；`auto` 下 `shell` 可读取系统与访问网络，作用域含子代理会话，副作用边界已在审批策略表列明。
- 数据目录被桌面端占用时 CLI 无法启动（既有 `host.lock` 独占设计），本规范不改变该约束。
- `--json` 输出格式为 CLI 内部契约（终行 schema 已定义），SDK 消费方如需稳定契约需另行登记。
- diff 渲染依赖 `similar`，现位于 `harness-platform` 共享模块（CLI re-export，见[右侧面板规范](side-panel-spec.md)）。
