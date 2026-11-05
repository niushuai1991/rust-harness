# CLI 命令行 Agent 调用计划

拟制日期：2026-11-05。

状态：已实施（2026-11-05，实施记录见文末）。

依据：用户要求参考 OpenAI Codex CLI（本地仓库 `D:\code\codex`）为本项目提供命令行 Agent 调用能力；经对现有 `harness-cli`、`harness-runtime` 与 codex CLI 架构调研后拟制本计划，按 TDD 实施。2026-11-05 依据两轮审查意见修订，见文末[修订记录](#修订记录)。

## 背景与目标

当前 `harness` 二进制（`crates/harness-cli`）只提供 JSON-RPC 接口（`rpc` stdio 长连接、`call` 单次调用），在命令行完成一次 Agent 任务需要 SDK 或手工构造 JSON-RPC 报文。本计划为 `harness` 增加直接的命令行能力：

- `run`：一次性执行任务，流式输出过程与结果后退出（对标 `codex exec`）。
- `chat`：终端交互式循环对话（简化版交互模式）。
- `diff` / `apply`：审阅隔离工作副本的具体改动并同步回原项目，补全项目核心工作流的命令行闭环。
- `sessions` / `models`：会话查看与模型管理。

## 设计原则（借鉴 codex CLI）

- 宿主薄、core 厚：CLI 子命令只是 `harness-runtime` 的薄宿主，内部复用现有 `dispatch()` 方法层（相当于 codex 的内嵌 app-server），不在 CLI 重复实现业务逻辑；个别 CLI 专属设置（如 `cli.default_model`）经 `Store::get/set` 直接读写，并在本计划中显式列出，不虚称"全部走 dispatch"。
- 单一事件流、多种渲染器：所有输出都是对 `Runtime::subscribe()` 的 `LiveEvent` 流的消费策略差异，参考 codex `EventProcessor` trait 抽象出人类可读与 JSONL 两种渲染器。
- 协议与宿主解耦的既有格局不变：桌面端继续走 Tauri IPC，CLI 与 SDK 继续共用 JSON-RPC 方法语义。
- CLI 需要而 runtime 缺失的能力，以最小幅度的 runtime 增强补齐（见下节），并保持既有消费者语义不变。

## runtime 增强（本计划包含的最小改动）

CLI 的作用域隔离无法纯客户端实现：runtime 在写入 `inbox.queued`（含私有 `message_id`）之后才等待会话队列锁，获得锁后才另行生成 `run_id` 并写入 `turn.start`，而 `session.send` 只返回 `null`，调用方无法证明"下一条 `turn.start` 属于自己发送的消息"。为此对本计划新增以下 runtime 改动（桌面端与 SDK 消费者语义均不变）：

1. **拆分入队与执行**：将 `run_session_with_attachments` 拆为
   - `Runtime::enqueue_message(session, text, attachments) -> AppResult<String>`：输入校验、附件校验与 `inbox.queued` 落库，立即返回 `message_id`；
   - `Runtime::execute_message(session, message_id, provider) -> AppResult<()>`：获取会话队列锁 → 取消检查 → `turn.start` → `inbox.claimed` → `user.message` → `steps()` → `turn.end` → `Status` 广播。
   - 原函数保留为两者的顺序组合包装，签名与"调用即等待回合结束"的语义不变；`dispatch` 的 `session.send` 与 orchestration 后台任务继续走包装函数。
2. **回合事件携带 `messageId`**：`turn.start`/`turn.end` 载荷在既有 `runId` 基础上增加 `messageId` 键。旧消费者忽略未知键，向后兼容。
3. **排队消息取消**：新增 `Runtime::cancel_message(session, message_id)` 登记"待取消消息"；`execute_message` 获得队列锁后检查命中，命中则落 `inbox.cancelled {"id":messageId}` 并以 `CANCELLED` 结束——不产生 `turn.start`/`turn.end`、不调用模型，避免取消误伤排队在自己前面的其他回合。

以上改动均在 `crates/harness-runtime/src/agent.rs`，实施时按 TDD 先补 `harness-runtime` 失败测试（拆分 API、`messageId` 断言、`cancel_message` 路径），并回归桌面端命令与 SDK。

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

1. **REPL 形态**：简单 readline 循环（`tokio::io::stdin().lines()`）加流式纯文本输出，不引入 ratatui 全屏 TUI；体验升级留待后续计划。
2. **默认模型**：优先级为 `-m/--model` > settings 键 `cli.default_model`（经 `Store::get/set` 读写）> 报错并列出可用模型。`models default <id>` 设置前经 `models.list_models()` 校验存在，不存在拒绝保存；`models default`（无参）查看当前值；`models default --clear` 清除（写 `Null` 视为未设置）。
3. **参数解析**：引入 `clap 4`（derive），替换 `main.rs` 手写解析；`rpc` 与 `call` 行为保持不变（以现有 `tests/rpc.rs` 与新增 `call` 冒烟测试回归）。
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

harness chat             # readline 循环；斜杠命令 /model /sessions /diff /apply /approval /quit

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
└── （本计划的最小 runtime 增强：enqueue/execute 拆分、turn 事件含 messageId、cancel_message）

crates/harness-cli/
├── Cargo.toml          # 新增 clap 4（workspace 依赖同步）、similar（diff 渲染，仅本 crate）
├── src/main.rs         # 重写：clap Parser + 子命令分发；保留 --data-dir 解析逻辑
├── src/lib.rs          # 现有 JSON-RPC dispatch/serve 不动
├── src/console.rs      # 新增：渲染器（human：正文 stdout、工具/审批/推理 stderr；json：逐行 JSON）
├── src/approval.rs     # 新增：auto|ask|never 策略，作用域集合内审批处理
├── src/scope.rs        # 新增：messageId 认领、子代理作用域集合、Lagged 降级
├── src/diffview.rs     # 新增：unified diff 渲染、kind 语义、阈值、截断、颜色
├── src/run.rs          # 新增：一次性执行（初始化规则 → enqueue → 事件消费 → 同 messageId turn.end）
├── src/chat.rs         # 新增：REPL（复用 run 的单轮执行内核）
├── src/manage.rs       # 新增：sessions/diff/apply/models
└── tests/run.rs        # 新增集成测试
```

## 实施步骤（TDD）

1. **runtime 增强**：先在 `harness-runtime` 补失败测试（拆分 API、`turn.start`/`turn.end` 含 `messageId`、`cancel_message` 的 `inbox.cancelled` 路径），实现后 `cargo test -p harness-runtime` 并回归桌面端命令与 SDK。
2. **clap 重构**：`main.rs` 换用 clap，`rpc`/`call` 行为不变；现有 `tests/rpc.rs` 保持通过，另补一条 `call` 子命令冒烟测试。
3. **作用域与 run 模式**：先写失败的集成测试——参考 `crates/harness-runtime/tests/http_protocols.rs` 的本地 SSE mock 服务器模式（`tests/rpc.rs` 是子进程 stdio JSON-RPC 回归，仅作 rpc/call 参考，不含 SSE mock），`provider.save`/`model.save` 指向 mock，子进程执行 `run`，断言流式输出、事件隔离与退出码；再实现 `scope.rs`、`console.rs`、`approval.rs`、`run.rs`。
4. **审批**：实现三模式及默认规则，测试见"测试计划"。
5. **diff/apply**：实现 `diffview.rs` 与交互 apply，测试渲染边界与错误路径。
6. **manage 命令**：`sessions list/show`、`models list/test/default`（default 经 `Store::get/set`，含校验与清除）。
7. **chat REPL**：复用 run 单轮内核加斜杠命令与单一 stdin 约定。

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

## 测试计划

关键场景（均纳入 `cargo test -p harness-cli --all-features`；runtime 增强部分先纳入 `cargo test -p harness-runtime`）：

- **runtime 增强**：enqueue/execute 拆分后包装函数行为不变；`turn.start`/`turn.end` 含 `messageId`；`cancel_message` 命中时落 `inbox.cancelled` 且不产生回合事件、不调用模型。
- **回合归属**：同会话存在排队回合时（预先占用队列锁构造），CLI 只认领自己 `messageId` 的回合；他人 `turn.end` 不提前结束本 run、不触发渲染或审批。
- **排队阶段取消**：消息排队期间 Ctrl+C → `cancel_message` → `inbox.cancelled` → 退出码 130，不影响正在执行的其他回合。
- **子代理作用域**：`agent_spawn` 后子会话的审批被 `auto` 放行 / `ask` 标注"子代理"并等待确认；父括号关闭后收尾阶段等待子会话结束（`is_active` 归零）。
- **审批作用域**：本会话 `ask` 输入 `n` 拒绝并得到 `APPROVAL_DENIED`；作用域外的待审批项不被触碰。
- **Lagged 降级**：注入 Lagged 后 durable 回补完整、stderr/event_gap 声明缺口、后续 `assistant.message` 打印全文、终行 result 仍完整。
- **正文去重**：仅 reasoning Delta 时 `assistant.message` 必须打印；正文 Delta 正常时抑制。
- **stdin 矩阵**：逐格验证启动/执行行为；chat 非 TTY 启动报错。
- **JSONL 契约**：SSE mock 驱动 `run --json`，断言 stdout 每行可解析、终行字段完整、无其他杂质。
- **diff 渲染**：added/deleted 空侧语义、modified 二进制提示、1 MiB 阈值、2000 行截断、颜色开关、CONFLICT 标注。
- **apply 错误路径**：`--all`/`a` 排除冲突文件且不整体失败；哈希过期（`STALE_DIFF`）整体失败不静默重试。
- **models default**：不存在的模型拒绝保存；清除后 run 报错列出可用模型。
- **rpc/call 回归**：`tests/rpc.rs` 保持通过；`call` 冒烟测试新增。

## 验证方式

- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`。
- `cargo test -p harness-runtime`（runtime 增强）与 `cargo test -p harness-cli --all-features`（上述测试计划）全绿。
- 配置真实模型后端到端手工验证：
  - `cargo run -p harness-cli -- run "列出本项目根目录的文件并总结"`（流式输出与 diff 提示）。
  - `echo 任务 | cargo run -p harness-cli -- run --json --approval auto`（管道模式须显式审批策略；JSONL 输出）。
  - `harness diff <workspaceId>` → `harness apply <workspaceId>` 审阅同步闭环。
  - `harness chat` 交互对话、`/diff`、`/quit`。
  - 桌面端同时运行时 `host.lock` 报错提示清晰。
  - 带子代理与排队回合的真实任务抽查回合归属与审批提示。

## 风险点

- 本计划包含最小 runtime 增强（回合流程拆分、事件新增键、排队取消），触及 `harness-runtime` 核心路径，须以 `harness-runtime` 测试与桌面端回归先行兜底。
- `auto` 为显式选项且默认关闭（默认 `ask`）；`auto` 下 `shell` 可读取系统与访问网络，作用域含子代理会话，副作用边界已在审批策略表列明。
- 数据目录被桌面端占用时 CLI 无法启动（既有 `host.lock` 独占设计），计划不改变该约束。
- `--json` 输出格式为 CLI 内部契约（终行 schema 已定义），SDK 消费方如需稳定契约需另行登记。
- diff 渲染引入 `similar` 新依赖，仅限 `harness-cli` crate。

## 实施记录（2026-11-05）

分支 `feat/cli-agent-run`，按 TDD 分六步提交：

| 提交 | 内容 |
|---|---|
| 877bbd7 | runtime 消息入队与执行拆分（`enqueue_message`/`execute_message`/`cancel_message`），回合事件携带 `messageId`；`agent_flow.rs` 预置 clippy 告警一并修复 |
| 961cd1c | `harness` 入口重构为 clap 子命令（rpc/call/run/chat/sessions/diff/apply/models），rpc 与 call 行为不变 |
| 8e9bc49 | `run` 一次性执行：messageId 认领作用域、子代理作用域集合、human/json 双渲染器、审批策略与 TTY 缺省规则、stdin 矩阵、脏副本拒绝、Ctrl+C 两段取消、退出码 0\|1\|130 |
| 16099b7 | `diff`/`apply`（kind 语义、1MiB 阈值、unified diff、冲突排除、y/n/a/q 交互）与 `sessions`/`models`（含 `cli.default_model` 校验与清除） |
| 62c075e | `chat` REPL：单轮内核 `turn_once` 复用、斜杠命令、单一 stdin 串行、非 TTY 报错 |
| 44c134a | `mcp.rs` 测试预置 clippy 告警修复（main 上即存在） |

测试覆盖（计划"测试计划"全部场景）：

- runtime 38 项（新增 `message_scoping.rs` 3 项：入队返回标识与回合携带 messageId、取消跳过且不调模型、取消只影响目标消息）。
- harness-cli 31 项：lib 单元 22 项（scope 认领/子代理/取消、console 去重/缺口/JSONL 契约/子代理前缀、approval TTY 规则、diffview added/deleted/二进制/阈值/截断/冲突排除/y-n-a-q/颜色、chat 斜杠命令、manage 默认模型校验）+ 子进程集成 9 项（SSE mock 驱动 JSONL 契约与终行、管道提示词、审批缺省报错、ask 需 TTY、模型未选择报错、call 冒烟、rpc 回归、chat 非 TTY）。
- 前端 35 项（worktree 需先 `npm ci`，根目录 node_modules 缺 react-markdown 属环境未同步）。

验证结果：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`（默认成员 84 项）、`cargo build --all-features`、`npm test` 全部通过。

与计划的偏差：无结构性偏差；run/chat 的复用警告在 execute 中直接输出 stderr（不进入渲染器）；`models list` 的 `ApiMode` 以 JSON 字符串展示（无 Display 实现）。

待人工验证：真实模型端到端（`run` 流式与 diff 提示、管道 `--json --approval auto`、`diff`/`apply` 闭环、`chat` 交互、子代理与排队回合任务抽查、桌面端 `host.lock` 互斥提示），见计划"验证方式"。

## 修订记录

- 2026-11-05（第二轮审查修订）：
  - **运行作用域重构**——原"纯客户端 runId 括号协议"无法证明回合归属（`inbox.queued` 在队列锁前写入、`run_id` 获锁后才生成），改为"runtime 增强 + messageId 认领"：新增 `enqueue_message`/`execute_message` 拆分、`turn.start`/`turn.end` 增加 `messageId`、`cancel_message` 排队取消；删除"无需改动 runtime"结论，取消契约按括号状态区分 `cancel_message`/`cancel`。
  - **子代理审批作用域**——`agent_spawn` 在子会话启动独立回合，父会话括号覆盖不到其审批；引入"作用域集合"（父会话 ∪ 递归后代，经 `subagent.created` 发现、`children_of` 兜底），审批与收尾判定作用于集合内所有会话。
  - **Lagged 降级**——`Delta`/`Status` 不持久化、不可回补，改为 durable 回补 + gap 声明（stderr 警告 / `event_gap` 行）+ 后续完整正文重建。
  - **正文去重修正**——按 `request.snapshot.id ↔ Delta.requestId` 关联且仅以 `reasoning=false` 的 Delta 判定；仅 reasoning Delta 或 Lagged 后强制打印全文。
  - **diff 语义修正**——按 `kind` 解释文本缺失（added→before 空、deleted→after 空、modified 单侧缺失才视为二进制）；1 MiB 前置阈值替代"生成后截断"；`--all`/`a` 与交互确认的选择集自动排除 `conflict=true`（`apply_inner` 对冲突选择整体返回 `CONFLICT`）。
  - **契约一致性**——JSON `result` 增加 `messageId` 且 `exitCode` 扩为 `0|1|130`；手工验证示例补 `--approval auto`；`--session` 与 `--workspace` 定义互斥；测试计划补充排队归属、排队取消、子代理作用域、Lagged、去重、diff 边界、冲突排除、chat TTY 检查等场景。
- 2026-11-05（首轮审查修订）：新增"运行作用域"、"stdin 与审批行为矩阵"、"diff 审阅与同步契约"、"输出与取消契约"、"测试计划"小节；审批默认改为 `ask` 并按平台报告修正安全论据；补工作区/会话/chat 初始化规则；`models default` 改为经 `Store::get/set` 并含校验与清除；修正 `tests/rpc.rs` 的引用描述与 `call` 回归说明。
