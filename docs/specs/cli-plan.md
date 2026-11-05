# CLI 命令行 Agent 调用计划

拟制日期：2026-11-05。

状态：计划待批准，未开始实施。

依据：用户要求参考 OpenAI Codex CLI（本地仓库 `D:\code\codex`）为本项目提供命令行 Agent 调用能力；经对现有 `harness-cli`、`harness-runtime` 与 codex CLI 架构调研后拟制本计划，待人工批准后按 TDD 实施。2026-11-05 依据审查意见修订，见文末[修订记录](#修订记录)。

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

## 运行作用域

事实约束：`Runtime::subscribe()` 是全部会话共享的 broadcast 通道（容量 4096）；`orchestration::install()` 会启动常驻调度器 tick，可能在任意会话产生事件与审批；同会话的回合经队列锁严格串行，但同一会话可能存在排队回合。

因此 run/chat 的渲染、完成判定与审批必须限定在"本次运行"内，采用 **session + runId 括号协议**（纯客户端实现，无需改动 runtime）：

1. 订阅后仅处理 `session_id` 匹配本会话的事件（`Delta`、`Status` 自带 `sessionId`；`Durable` 事件经 `SessionEvent.session_id` 判定）。
2. 发送消息后等待本会话的 `turn.start`，记录其 `runId` 作为本次括号（`turn.start`/`turn.end` 载荷已携带 `runId`）。
3. 流式渲染、完成判定、审批处理全部限定在括号内：只渲染括号内的 `Delta`；只把**同 `runId`** 的 `turn.end` 判为本轮结束；`auto` 审批只自动允许括号内本会话出现的 `approval.request`（载荷自带 `session_id`），不触碰其他会话的待审批项。
4. 队列串行保证括号内本会话事件均属本次运行；其他会话的任何事件（含 `turn.end`）不影响本 run。
5. broadcast `Lagged` 时，以 `store.events(session, after=最后已见 seq)` 回补缺口后继续。

## 审批策略

平台报告明确当前沙箱边界："WRITE_RESTRICTED+Low IL 提供真实系统写入边界，**不隔离读取和网络**"。隔离工作副本与 apply 人工审阅是"改动不进原项目"的边界，不是工具执行副作用的边界：`shell` 可读取系统文件并访问网络，`auto` 还会放行后台任务、定时任务与子代理。据此审批策略如下：

| 模式 | 行为 | 副作用边界 |
|---|---|---|
| `ask`（默认，stdin 为 TTY） | 终端展示工具名与参数，等待 y/n；600 秒无响应视为拒绝 | 与桌面端审批同边界 |
| `auto`（须显式选择） | 自动允许**本次括号内本会话**的全部审批请求 | `shell` 可读系统文件、访问网络；可拉起后台任务、定时任务、子代理；改动仍只落副本，进原项目需 `apply` |
| `never` | 一律拒绝需审批工具 | 仍可执行免审批白名单：`read`/`read_artifact`/`list_files`/`search`/`agent_status` 只读；`todo_write`/`plan_write`/`goal_write` 会持久写入会话事件库（非文件系统、不触碰副本与原项目） |

默认规则：`--approval` 缺省时，stdin 为 TTY 取 `ask`；stdin 非 TTY 时启动报错，要求显式传 `auto|ask|never`。`ask` 在 stdin 非 TTY 时一律启动报错。

## stdin 与审批行为矩阵

| 提示词来源 | stdin 类型 | `ask` | `auto`/`never` |
|---|---|---|---|
| 命令行 PROMPT | TTY | 正常执行；审批从 stdin 读 y/n | 正常执行 |
| 命令行 PROMPT | 管道/重定向 | 启动报错（stdin 非交互，无法确认） | 正常执行 |
| `--prompt -` | 任意 | 启动报错（同上） | 正常执行；提示词读 stdin 至 EOF |
| 缺省 PROMPT | TTY | 启动报错（对齐 codex exec：不挂起等待） | 同左 |
| 缺省 PROMPT | 管道 | — | 读 stdin 至 EOF 作为提示词；输入为空报错 |

`chat` 的 stdin 消费约定：REPL 行读取与审批 y/n 读取是**同一 stdin 的串行消费者**——回合进行中 REPL 不读取输入，审批完成后才回到提示符；EOF 或 `/quit` 退出。

## 工作区与会话初始化规则

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
  --workspace <id>       # 显式指定工作区，跳过路径匹配与多工作区歧义
  --session <id>         # 续已有会话；缺省新建（标题取提示词前缀）
  --prompt -             # 从 stdin 读提示词至 EOF
  --json                 # JSONL 事件流输出到 stdout，契约见"输出与取消契约"
  --approval auto|ask|never
  --color auto|always|never
  PROMPT 缺省时按 stdin 矩阵处理
  退出码：同 runId 的 turn.end 为 completed 返回 0；error、cancelled 或发送失败返回 1；Ctrl+C 取消返回 130
  结束时副本存在改动则在 stderr 提示 diff/apply 命令

harness chat             # readline 循环；斜杠命令 /model /sessions /diff /apply /approval /quit

harness sessions [list|show <id>]
  list 走 dispatch session.list；show 打印会话信息与尾部事件摘要（store.session + session.events）

harness diff <workspaceId> [--color ...]
  渲染契约见"diff 审阅与同步契约"

harness apply <workspaceId> [--all]
  交互逐文件 y/n/a/q（a=全部剩余，q=中止）；--all 跳过确认；契约见下节

harness models [list|test <id>|default [<id>|--clear]]
（保留 rpc / call）
```

## diff 审阅与同步契约

`FileChange` 携带 `path/kind/before_hash/after_hash/source_hash/conflict/before_text/after_text`，审阅输出必须让用户看到具体修改内容：

- 渲染：对 `before_text`/`after_text` 生成 unified diff（引入 `similar` crate，仅 `harness-cli` 依赖）。
- 文本缺失（二进制或超大文件）→ 不渲染正文，显示 path/kind 与 before/after 哈希提示。
- `conflict=true` → 显式 `CONFLICT` 标识并着色，apply 交互中默认置为不选。
- 截断：单文件 diff 超过 2000 行截断并提示。
- 颜色：`--color auto|always|never`，默认 auto（非 TTY 不着色）。
- apply：确认清单来自 `workspace.diff` 返回的完整 `FileChange` 快照；提交 `apply_reviewed` 时保持哈希一致，其内置的哈希重校验失败（外部修改/过期）按原错误整体呈现，不做静默重试。

## 模块结构

```text
crates/harness-cli/
├── Cargo.toml          # 新增 clap 4（workspace 依赖同步）、similar（diff 渲染，仅本 crate）
├── src/main.rs         # 重写：clap Parser + 子命令分发；保留 --data-dir 解析逻辑
├── src/lib.rs          # 现有 JSON-RPC dispatch/serve 不动
├── src/console.rs      # 新增：渲染器（human：正文 stdout、工具/审批/推理 stderr；json：逐行 JSON）
├── src/approval.rs     # 新增：auto|ask|never 策略，括号内审批处理
├── src/scope.rs        # 新增：session + runId 括号协议与 Lagged 回补
├── src/diffview.rs     # 新增：unified diff 渲染、截断、颜色
├── src/run.rs          # 新增：一次性执行（初始化规则 → 事件消费 → session.send → 等待同 runId turn.end）
├── src/chat.rs         # 新增：REPL（复用 run 的单轮执行内核）
├── src/manage.rs       # 新增：sessions/diff/apply/models
└── tests/run.rs        # 新增集成测试
```

## 实施步骤（TDD）

1. **clap 重构**：`main.rs` 换用 clap，`rpc`/`call` 行为不变；现有 `tests/rpc.rs` 保持通过，另补一条 `call` 子命令冒烟测试。
2. **作用域与 run 模式**：先写失败的集成测试——参考 `crates/harness-runtime/tests/http_protocols.rs` 的本地 SSE mock 服务器模式（`tests/rpc.rs` 是子进程 stdio JSON-RPC 回归，仅作 rpc/call 参考，不含 SSE mock），`provider.save`/`model.save` 指向 mock，子进程执行 `run`，断言流式输出、事件隔离与退出码；再实现 `scope.rs`、`console.rs`、`approval.rs`、`run.rs`。
3. **审批**：实现三模式及默认规则，测试见"测试计划"。
4. **diff/apply**：实现 `diffview.rs` 与交互 apply，测试渲染边界与错误路径。
5. **manage 命令**：`sessions list/show`、`models list/test/default`（default 经 `Store::get/set`，含校验与清除）。
6. **chat REPL**：复用 run 单轮内核加斜杠命令与单一 stdin 约定。

## 输出与取消契约

- 人类模式：`Delta` 正文（`reasoning=false`）→ stdout；工具调用摘要、审批提示、错误、reasoning → stderr。`assistant.message` 持久事件不重复打印：以"自上一条 `assistant.message` 以来是否收到过本会话 `Delta`"判定（其载荷不含 `requestId`）；整个回合零 Delta（如 lag 回补）时补打全文一次。
- JSON 模式（`--json`）：stdout 仅输出 JSONL 且每行必须可独立解析：
  - `{"type":"delta","sessionId","requestId","index","text","reasoning"}`
  - `{"type":"event","event":<SessionEvent>}`
  - 终行 `{"type":"result","runId","sessionId","workspaceId","status":"completed|error|cancelled","exitCode":0|1,"finalMessage":<string>,"changedFiles":<n>}`
  - 人类可读摘要（含 diff 提示）一律 stderr，保证 stdout 可安全管道消费。
- 取消：首次 Ctrl+C → `runtime.cancel(sessionId)`，等待同 `runId` 的 `turn.end`（≤10 秒）后以退出码 130 退出；再次 Ctrl+C 立即以 130 退出。

## 测试计划

关键场景（均纳入 `cargo test -p harness-cli --all-features`）：

- **跨会话/跨回合隔离**：同一 runtime 两个会话并发回合，本 run 只响应自身 `runId` 括号；另一会话的 `turn.end` 不提前结束本 run。
- **审批作用域**：本会话 `ask` 输入 `n` 拒绝并得到 `APPROVAL_DENIED`；存在其他会话待审批项时，`auto` 不触碰它们。
- **stdin 矩阵**：逐格验证启动/执行行为。
- **JSONL 契约**：SSE mock 驱动 `run --json`，断言 stdout 每行可解析、终行字段完整、无其他杂质。
- **diff 渲染**：unified diff 正确性、二进制提示、`CONFLICT` 标识、截断、颜色开关。
- **apply 错误路径**：哈希过期/冲突时整体失败，不静默重试。
- **models default**：不存在的模型拒绝保存；清除后 run 报错列出可用模型。
- **rpc/call 回归**：`tests/rpc.rs` 保持通过；`call` 冒烟测试新增。

## 验证方式

- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`。
- `cargo test -p harness-cli --all-features`：上述测试计划全绿。
- 配置真实模型后端到端手工验证：
  - `cargo run -p harness-cli -- run "列出本项目根目录的文件并总结"`（流式输出与 diff 提示）。
  - `echo 任务 | cargo run -p harness-cli -- run --json`（JSONL 输出）。
  - `harness diff <workspaceId>` → `harness apply <workspaceId>` 审阅同步闭环。
  - `harness chat` 交互对话、`/diff`、`/quit`。
  - 桌面端同时运行时 `host.lock` 报错提示清晰。

## 风险点

- `auto` 为显式选项且默认关闭（默认 `ask`）；`auto` 下 `shell` 可读取系统与访问网络，副作用边界已在审批策略表列明。
- 数据目录被桌面端占用时 CLI 无法启动（既有 `host.lock` 独占设计），计划不改变该约束。
- `--json` 输出格式为 CLI 内部契约（终行 schema 已定义），SDK 消费方如需稳定契约需另行登记。
- diff 渲染引入 `similar` 新依赖，仅限 `harness-cli` crate。

## 修订记录

- 2026-11-05：依据审查意见修订——新增"运行作用域"（session + runId 括号协议）、"stdin 与审批行为矩阵"、"diff 审阅与同步契约"、"输出与取消契约"、"测试计划"小节；审批默认改为 `ask` 并按平台报告修正安全论据；补工作区/会话/chat 初始化规则；`models default` 改为经 `Store::get/set` 并含校验与清除；修正 `tests/rpc.rs` 的引用描述与"无需改动 runtime"的依据（复用既有 `runId`/`request_id` 字段，纯客户端实现作用域隔离）。
