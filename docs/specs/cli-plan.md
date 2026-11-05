# CLI 命令行 Agent 调用计划

拟制日期：2026-11-05。

状态：计划待批准，未开始实施。

依据：用户要求参考 OpenAI Codex CLI（本地仓库 `D:\code\codex`）为本项目提供命令行 Agent 调用能力；经对现有 `harness-cli`、`harness-runtime` 与 codex CLI 架构调研后拟制本计划，待人工批准后按 TDD 实施。

## 背景与目标

当前 `harness` 二进制（`crates/harness-cli`）只提供 JSON-RPC 接口（`rpc` stdio 长连接、`call` 单次调用），在命令行完成一次 Agent 任务需要 SDK 或手工构造 JSON-RPC 报文。本计划为 `harness` 增加直接的命令行能力：

- `run`：一次性执行任务，流式输出过程与结果后退出（对标 `codex exec`）。
- `chat`：终端交互式循环对话（简化版交互模式）。
- `diff` / `apply`：审阅隔离工作副本改动并同步回原项目，补全项目核心工作流的命令行闭环。
- `sessions` / `models`：会话查看与模型管理。

## 设计原则（借鉴 codex CLI）

- 宿主薄、core 厚：CLI 子命令只是 `harness-runtime` 的薄宿主，内部复用现有 `dispatch()` 方法层（相当于 codex 的内嵌 app-server），不在 CLI 重复实现业务逻辑。
- 单一事件流、多种渲染器：所有输出都是对 `Runtime::subscribe()` 的 `LiveEvent` 流的消费策略差异，参考 codex `EventProcessor` trait 抽象出人类可读与 JSONL 两种渲染器。
- 协议与宿主解耦的既有格局不变：桌面端继续走 Tauri IPC，CLI 与 SDK 继续共用 JSON-RPC 方法语义。

## 复用的既有机制（无需改动 runtime）

| 机制 | 位置 | 用途 |
|---|---|---|
| `LiveEvent::{Durable, Delta, Status}` | `crates/harness-contracts/src/lib.rs` | 流式增量与事件回放的统一订阅源 |
| `approval.request` 事件 + `Approvals::resolve(id, allow)` | `crates/harness-runtime/src/agent.rs` | CLI 审批策略的落点；待审批等待 600 秒，拒绝返回 `APPROVAL_DENIED` |
| `workspaces.list()/create()/diff()/apply_reviewed()` | `crates/harness-platform/src/workspace.rs` | `WorkspaceInfo.source_path` 支持按路径匹配复用工作区 |
| `models.list_models()/build_model()/test_model()` | `crates/harness-runtime/src/providers.rs` | 模型选择与连接测试；凭据从 Windows 凭据管理器读取 |
| `Store::get()/set()` | `crates/harness-storage/src/lib.rs` | 存储 `cli.default_model` 设置 |
| 本地 SSE 模拟测试模式 | `crates/harness-runtime/tests/http_protocols.rs`、`crates/harness-cli/tests/rpc.rs` | run 模式集成测试的参考实现 |

运行时核心不需要新增改动；如实施中发现缺口，在本计划修订后再评估。

## 设计决策

1. **范围**：`run`（核心）+ `diff`/`apply` + `models`/`sessions` + `chat` 简易 REPL，一个功能分支交付。
2. **审批策略**：`--approval auto|ask|never` 三种模式，默认 `auto`。理由：隔离工作副本与受限 shell 沙箱已是双重安全网，改动仍需 `diff`/`apply` 人工确认才会进入原项目；CLI 定位为无人值守自动化，交互式审批场景仍归桌面端。`ask` 模式在终端展示工具名与参数后等待 y/n；stdin 非 TTY 时 `ask` 直接报错，避免卡到 600 秒超时。`never` 拒绝所有需审批工具，仅保留只读能力。
3. **REPL 形态**：简单 readline 循环（`tokio::io::stdin().lines()`）加流式纯文本输出，不引入 ratatui 全屏 TUI；体验升级留待后续计划。
4. **默认模型**：优先级为 `-m/--model` 参数 > settings 键 `cli.default_model`（`harness models default <id>` 设置）> 报错并列出可用模型。
5. **参数解析**：引入 `clap 4`（derive），替换 `main.rs` 手写解析；`rpc` 与 `call` 行为保持不变，现有测试继续通过。
6. **数据目录**：维持现状（`--data-dir`、环境变量 `RUST_HARNESS_DATA_DIR`、默认 `%LOCALAPPDATA%\RustHarness`），`host.lock` 独占与桌面端互斥的关系不变。

## 命令规格

```text
harness run [PROMPT]     # 别名 e；一次性执行，流式输出，结束退出
  -C/--cwd <path>        # 目标项目目录，默认当前目录；canonicalize 后按 source_path 复用已有工作区，否则创建
  -m/--model <id>        # 模型选择，见默认模型决策
  --session <id>         # 续已有会话；缺省新建（标题取提示词前缀）
  --json                 # JSONL 事件流输出到 stdout（LiveEvent 序列化 + 最终 result 行）
  --approval auto|ask|never
  PROMPT 缺省时从 stdin 读取（支持管道）
  退出码：turn.end 为 completed 返回 0；error、cancelled 或发送失败返回 1
  结束时副本存在改动则在 stderr 提示 diff/apply 命令

harness chat             # readline 循环；斜杠命令 /model /sessions /diff /apply /approval /quit

harness sessions [list|show <id>]
harness diff <workspaceId>        # FileChange 列表（path/kind 摘要）
harness apply <workspaceId>       # 逐项交互确认或 --all；调用 apply_reviewed
harness models [list|test <id>|default <id>]
（保留 rpc / call）
```

## 模块结构

```text
crates/harness-cli/
├── Cargo.toml          # 新增 clap 4（workspace 依赖同步）
├── src/main.rs         # 重写：clap Parser + 子命令分发；保留 --data-dir 解析逻辑
├── src/lib.rs          # 现有 JSON-RPC dispatch/serve 不动
├── src/console.rs      # 新增：渲染器（human：正文 stdout、工具/审批/推理 stderr；json：逐行 JSON）
├── src/approval.rs     # 新增：auto|ask|never 策略，订阅 approval.request 后调用 resolve
├── src/run.rs          # 新增：一次性执行（工作区/会话解析 → 事件消费 → session.send → 等待 turn.end）
├── src/chat.rs         # 新增：REPL（复用 run 的单轮执行内核）
├── src/manage.rs       # 新增：sessions/diff/apply/models（内部直接调 dispatch()）
└── tests/run.rs        # 新增集成测试
```

## 实施步骤（TDD）

1. **clap 重构**：`main.rs` 换用 clap，`rpc`/`call` 行为不变，`tests/rpc.rs` 仍通过。
2. **run 模式**：先写失败的集成测试——参考 `http_protocols.rs` 起本地 SSE mock 服务器，`provider.save`/`model.save` 指向它，子进程执行 `run`，断言流式输出与退出码；再实现 `console.rs`、`approval.rs`、`run.rs`。
3. **审批测试**：auto 直接放行；ask 依 stdin 输入 y/n；never 收到 `APPROVAL_DENIED` 且退出码为 1。
4. **manage 命令**：内部复用 `dispatch()` 已有方法，测试从简。
5. **chat REPL**：复用 run 单轮内核加斜杠命令。

## 验证方式

- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`。
- `cargo test -p harness-cli --all-features`：新增 run、审批、manage 集成测试全绿。
- 配置真实模型后端到端手工验证：
  - `cargo run -p harness-cli -- run "列出本项目根目录的文件并总结"`（流式输出与 diff 提示）。
  - `echo 任务 | cargo run -p harness-cli -- run --json`（JSONL 输出）。
  - `harness diff <workspaceId>` → `harness apply <workspaceId>` 审阅同步闭环。
  - `harness chat` 交互对话、`/diff`、`/quit`。
  - 桌面端同时运行时 `host.lock` 报错提示清晰。

## 风险点

- 默认 `auto` 审批意味着 CLI 无人值守时会自动执行写副本与受限 shell 操作；安全边界依赖副本隔离与 apply 人工确认，如需保守可改默认为 `ask`。
- 数据目录被桌面端占用时 CLI 无法启动（既有 `host.lock` 独占设计），计划不改变该约束。
- `--json` 输出格式为 `LiveEvent` 逐行 JSON 加最终 result 行，属于 CLI 内部约定，SDK 消费方如需稳定契约需另行登记。
