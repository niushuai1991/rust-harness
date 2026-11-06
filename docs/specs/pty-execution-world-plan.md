# PTY 执行世界（本地沙箱终端）计划

计划拟制日期：2026-11-06。

## 背景与范围

P2 阶段验收项中的「PTY」尚未开工（能力矩阵记为未实现）。当前全部 Agent 进程经 `harness-platform::shell::windows` 以受限 token + Low IL + 能力 SID ACL + Job Object + 匿名管道执行，输出一次性全量收集（16MiB 截断）：前台 `shell` 工具与后台 `background_run` 均无法运行交互式程序（REPL、调试器、需要 stdin 确认的安装器等）。

本计划交付本地沙箱 PTY 终端能力：模型可打开持久交互式终端，写入命令并增量读取输出。完成即计 P2 的 PTY 项。

不在本次范围：SSH 远端执行世界（P3，仅接口 seam 预留）、桌面 xterm.js 交互终端面板、屏幕仿真与 prompt 就绪检测、终端 resize 的模型接口、跨会话终端管理面板、非 Windows 平台实现（返回 `UNSUPPORTED`）。

## 设计

### PTY 原语（harness-platform，自研 ConPTY）

不引入 portable-pty：项目安全不变量要求子进程经 `CreateProcessAsUserW`（受限 token）启动，portable-pty 默认后端做不到。新增 `pty` 模块，仅给 windows-sys 增加 `Win32_System_Console` feature：

- `PtyService::spawn(root, cancel) -> SandboxedPty`：`CreatePipe` 两对 → `CreatePseudoConsole(120x30)` → `STARTUPINFOEXW` + `UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE)` → 复用 shell/windows 的 `token()/grant()/capability()` 以 `CreateProcessAsUserW(CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT)` 启动 → `AssignProcessToJobObject(KILL_ON_JOB_CLOSE)` → `ResumeThread`。ConPTY 下不传 `CREATE_NO_WINDOW`。
- Shell 选择与 `run()` 一致：pwsh 7 回退 Windows PowerShell，`-NoLogo -NoProfile` 交互模式，cwd 为副本 root；env 沿用「父环境去 TEMP/TMP → 私有 temp」。
- `SandboxedPty`：异步 input writer；输出泵为阻塞读线程（16KB 块、UTF-8 边界安全解码，直接写入共享 OutputBuffer，消费方自行聚合）；`resize`；exit waiter 线程上报退出码；guard drop 兜底 `TerminateJobObject`。捐赠给 `CreatePseudoConsole` 的两端句柄保留至会话结束（portable-pty/codex 在 Windows 上的既有做法）。杀树只靠 Job Object；Windows 中断（SIGINT 语义）= 向输入写 `\r`（`\n` 归一化为 `\r`，控制台输入处理器以 CR 为回车）。
- 纯函数单测覆盖 ANSI 控制序列过滤器（CSI/OSC 剥离、跨块转义边界）与有界缓冲（尾部 ring + 截断标志、消费式增量读取）；真实 ConPTY 集成测试参照 `tests/sandbox.rs` 真实进程测试形态。

### 终端管理器与模型工具（harness-orchestration）

`terminals.rs` 定义 `PtyWorld` trait（`spawn(root, cancel) -> TerminalSession`；`TerminalSession`：`write/read/exit/terminate`）作为执行世界 seam——P3 SSH 远端实现同一 trait 接入。真实实现 `ConPtyWorld` 包装 platform 原语；`install` 默认真实后端，`install_with` 供测试注入 stub。

工具四件套（注册进既有 `install` 循环）：

| 工具 | 参数 | 审批 | 行为 |
|---|---|---|---|
| `terminal_open` | 无 | **需审批** | 每会话上限 4 个；acquire 工作区租约并持有至关闭；spawn 交互式 shell；追加 `terminal.opened {id, shell}`；返回 `{id}` |
| `terminal_write` | `{id, input, yieldMs?}` | 免 | 写入 stdin；等待 `yieldMs`（缺省 1000，钳制 250–30000ms）收集新输出；返回 `{id, output, status}`；status 含退出码 |
| `terminal_read` | `{id, yieldMs?}` | 免 | 不写只读，同上返回增量 |
| `terminal_close` | `{id}` | 免 | terminate 杀树、等退出、追加 `terminal.closed`、释放租约；返回 `{id, closed:true}` |

- 输出可见性：读侧返回「自上次读取以来的增量」——ANSI 转义过滤后的 lossy UTF-8 文本，单次上限 64KiB；终端退出时尾部缓冲（128KiB ring + 截断标志）经 `put_blob` spill，`terminal.closed {id, status, exitCode?, outputBlobHash?}` 记录 hash，模型用 `read_artifact` 取回。
- 作用域与恢复：沿后台 job 模式——write/read/close 校验会话归属（`TERMINAL_SCOPE`）；安装后扫描 opened 无 closed 的会话追加 `terminal.closed {status:"interrupted"}`（副作用不重放）；`State.stop` 级联取消杀树。
- 审批语义（风险记录）：`terminal_open` 一次审批覆盖该终端后续全部输入，不再逐命令审批——交互式终端无法逐击键审批；一次性命令仍走 `shell`（每命令审批）。工具描述向模型写明分工：交互式程序用 terminal_*，一次性命令用 shell。

### 事件卡片（frontend/App.tsx `EventCard`）

`terminal.opened`（终端 id + shell + 运行中状态）、`terminal.closed`（终态 + 退出码/中断说明，展开可看 blob hash）。只读呈现，无操作入口；不新增 Tauri 命令，bindings 不变。

## 边界

- 终端输出不全文落事件存储：`tool.call/tool.result` 记录每次读写往来，`terminal.closed` 记录尾部 blob；完整转写不保证。
- 会话取消（回合 cancel）不杀终端——终端跨回合存续，与后台 job 一致；只有 `terminal_close`、应用关闭或杀树语义会终结。
- 事件流推送沿用 `session_send` 活跃期广播，无额外推送通道。
- 环境边界（2026-11-06 实测）：开发机（Windows 11 Insider 26300）上 ConPTY 客户端连接失效——`CreatePseudoConsole` 宿主存活但子进程一律以 0xC0000142 立即退出、输出管道零字节，最小 MS 样例探针与受限 token/裸进程/计划任务上下文均复现，非 ConPTY 新控制台（`start`）正常；参考项目 deepseek-harness 记录了同类结论（console isolation unavailable，pipe stdio 不受影响）。故真实 ConPTY 集成测试带可用性预检：不可用时打印原因跳过（与浏览器测试无浏览器环境的处理一致），真实验证在 GitHub CI windows runner 上进行；桌面端在 ConPTY 不可用的机器上 `terminal_open` 将返回明确错误。

## 验证

- Rust：平台 ANSI/缓冲纯函数单测 + ConPTY 真实进程集成测试；orchestration stub 后端全流程（审批矩阵、open/write/read/close、4 上限、跨会话拒绝、租约存续与释放、重启 interrupted、事件完整性）；`cargo fmt --check`、`clippy --all-targets --all-features -- -D warnings`、`test --all-features`、`build --all-features`。
- 前端：EventCard terminal.* 渲染测试；`npm test`、`npm run build`。
- 真实模型端到端：CLI + SQLite 快照独立数据目录 + 凭据管理器复用 + `--approval auto`，模型完成 open→write（交互变量赋值后输出）→read→close 并在回复中报告输出；重启后未关闭终端标 interrupted。
- 桌面端卡片抽查留给用户。
