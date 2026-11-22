# PTY 执行世界（本地交互终端）规范

- 日期：2026-11-06（计划拟制）；修订：2026-11-22（终端扩展到全部执行位置：worktree/scratch 沙箱内可用，直接模式宿主令牌 + 每条输入审批；弃用 `sandbox_lease` 路径反推）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：ConPTY 持久交互终端，`terminal_open/write/read/close` 工具组；自研 ConPTY + 受限 token/宿主令牌 spawn；P3 SSH 远端仅留 trait seam。

## 背景与范围

P2 阶段验收项中的「PTY」。当前全部 Agent 进程经 `harness-platform::shell::windows` 执行，输出一次性全量收集（16MiB 截断）：前台 `shell` 工具与后台 `background_run` 均无法运行交互式程序（REPL、调试器、需要 stdin 确认的安装器等）。

本规范约定本地 PTY 终端能力：模型可打开持久交互式终端，写入命令并增量读取输出。2026-11-22 起终端在四种执行位置均可用（此前仅隔离副本）。

不在范围：SSH 远端执行世界（P3，仅接口 seam 预留）、桌面 xterm.js 交互终端面板、屏幕仿真与 prompt 就绪检测、终端 resize 的模型接口、跨会话终端管理面板、非 Windows 平台实现（返回 `UNSUPPORTED`）。

## 设计

### PTY 原语（harness-platform，自研 ConPTY）

不引入 portable-pty：项目安全不变量要求子进程经受控方式启动。`pty` 模块，仅给 windows-sys 增加 `Win32_System_Console` feature：

- `PtyService::spawn(root, capability_id, cancel, sandboxed) -> SandboxedPty`：`CreatePipe` 两对 → `CreatePseudoConsole(120x30)` → `STARTUPINFOEXW` + `UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE)` → 沙箱内（副本/worktree/scratch）以 `capability_id` 派生受限令牌、`CreateProcessAsUserW` 启动；直接模式（`sandboxed=false`）以宿主令牌 `CreateProcessW` 启动 → `AssignProcessToJobObject(KILL_ON_JOB_CLOSE)` → `ResumeThread`。ConPTY 下不传 `CREATE_NO_WINDOW`。2026-11-22 修订：不再从路径反推项目（`sandbox_lease` 已随直接模式/后台任务改造移除），租约由调用方持有，临时目录在系统默认位置（与 `run_in` 一致）。
- Shell 选择与 `run_in` 一致：pwsh 7 回退 Windows PowerShell，`-NoLogo -NoProfile` 交互模式，cwd 为会话执行根；env 沿用「父环境去 TEMP/TMP → 私有 temp」。
- `SandboxedPty`：异步 input writer；输出泵为阻塞读线程（16KB 块、UTF-8 边界安全解码，直接写入共享 OutputBuffer，消费方自行聚合）；`resize`；exit waiter 线程上报退出码；guard drop 兜底 `TerminateJobObject`。捐赠给 `CreatePseudoConsole` 的两端句柄保留至会话结束。杀树只靠 Job Object；Windows 中断（SIGINT 语义）= 向输入写 `\r`（`\n` 归一化为 `\r`，控制台输入处理器以 CR 为回车）。
- 纯函数单测覆盖 ANSI 控制序列过滤器（CSI/OSC 剥离、跨块转义边界）与有界缓冲（尾部 ring + 截断标志、消费式增量读取）；真实 ConPTY 集成测试参照 `tests/sandbox.rs` 真实进程测试形态。

### 终端管理器与模型工具（harness-orchestration）

`terminals.rs` 定义 `PtyWorld` trait（`spawn(root, capability_id, cancel, sandboxed) -> TerminalSession`；`TerminalSession`：`write/read/exit/terminate`）作为执行世界 seam——P3 SSH 远端实现同一 trait 接入。真实实现 `PlatformWorld` 包装 platform 原语；`install` 默认真实后端，`install_with` 供测试注入 stub。

工具四件套（注册进既有 `install` 循环）：

| 工具 | 参数 | 审批 | 行为 |
|---|---|---|---|
| `terminal_open` | 无 | **需审批** | 每会话上限 4 个；acquire 工作区租约并持有至关闭（无项目会话不持）；spawn 交互式 shell；追加 `terminal.opened {id, shell}`；返回 `{id}` |
| `terminal_write` | `{id, input, yieldMs?}` | 沙箱内免；**直接模式每条审批** | 写入 stdin；等待 `yieldMs`（缺省 1000，钳制 250–30000ms）收集新输出；返回 `{id, output, status}`；status 含退出码 |
| `terminal_read` | `{id, yieldMs?}` | 免 | 不写只读，同上返回增量 |
| `terminal_close` | `{id}` | 免 | terminate 杀树、等退出、追加 `terminal.closed`、释放租约；返回 `{id, closed:true}` |

- 输出可见性：读侧返回「自上次读取以来的增量」——ANSI 转义过滤后的 lossy UTF-8 文本，单次上限 64KiB；终端退出时尾部缓冲（128KiB ring + 截断标志）经 `put_blob` spill，`terminal.closed {id, status, exitCode?, outputBlobHash?}` 记录 hash，模型用 `read_artifact` 取回。
- 作用域与恢复：沿后台 job 模式——write/read/close 校验会话归属（`TERMINAL_SCOPE`）；安装后扫描 opened 无 closed 的会话追加 `terminal.closed {status:"interrupted"}`（副作用不重放）；`State.stop` 级联取消杀树。
- 审批语义：沙箱内终端（副本/worktree/scratch，受限令牌）`terminal_open` 一次审批覆盖该终端后续全部输入，不再逐命令审批——交互式终端无法逐击键审批。**直接模式终端（宿主令牌）不同**：`terminal_open` 审批理由明确警示无沙箱，`terminal_write` 每条输入单独审批（对齐前台 shell 的审批节奏，避免「一次批准换整段免审命令」）。`Guarded` 档对 `terminal_open` 一律询问（见[权限模式规范](permission-modes.md)）。一次性命令仍走 `shell`。工具描述向模型写明分工：交互式程序用 terminal_*，一次性命令用 shell。
- 执行位置：终端跟随会话执行位置——副本（受限令牌，capability=工作区 id）、worktree（受限令牌，capability=会话 id，与目录授权同源）、scratch（受限令牌，capability=会话 id）、直接模式（宿主令牌）。此前仅副本可用（依赖已移除的路径反推检查）。

### 事件卡片（frontend/App.tsx `EventCard`）

`terminal.opened`（终端 id + shell + 运行中状态）、`terminal.closed`（终态 + 退出码/中断说明，展开可看 blob hash）。只读呈现，无操作入口；不新增 Tauri 命令，bindings 不变。

## 边界

- 终端输出不全文落事件存储：`tool.call/tool.result` 记录每次读写往来，`terminal.closed` 记录尾部 blob；完整转写不保证。
- 会话取消（回合 cancel）不杀终端——终端跨回合存续，与后台 job 一致；只有 `terminal_close`、应用关闭或杀树语义会终结。
- 事件流推送沿用 `session_send` 活跃期广播，无额外推送通道。
- ConPTY 可用性因机器而异：真实 ConPTY 集成测试带可用性预检，不可用时打印原因跳过（与浏览器测试无浏览器环境的处理一致）；桌面端在 ConPTY 不可用的机器上 `terminal_open` 返回明确错误。
- **退出码探测双语义**（2026-11-22）：`exit_code` 阻塞等待（`terminal_close` 的 10 秒超时兜底内使用）、`try_exit_code` 非阻塞（`terminal_write`/`terminal_read` 逐条返回依赖）。阻塞式 await 在进程存活期永久挂起，曾导致桌面真实抽查中每次写入卡死（tool.result 永不落库）。
- **ConPTY 附着的平台限制（2026-11-22 深度排查，根因链完整）**：真实桌面抽查发现 GUI 宿主下终端零输出后，用自包含矩阵探针（`examples/attach_matrix.rs` GUI 版 + `attach_matrix_console.rs` 控制台版对照）完成消元——失败的组合与 shell 种类（5.1/cmd）、`PSEUDOCONSOLE_INHERIT_CURSOR`、`CREATE_SUSPENDED`、默认终端委托（delegation）、Job Object、`CreateProcessW` vs `CreateProcessAsUserW`（宿主令牌）均无关；headless conhost 正常创建（`--server` 句柄存在）但子进程不连接。最终矩阵：
  - 受限令牌 + `CreateProcessAsUserW`：控制台宿主✅附着成功；GUI 桌面宿主 = 0xC0000142 立死（11 月以来历史会话数据一致）。
  - 宿主令牌（`CreateProcessW` 或 `AsUserW`）：控制台与 GUI 宿主均❌活着但零输出（ConPTY 管道无人消费）。
  - 结论：**Insider 26300 的 inbox conhost 在「GUI 宿主 × 任一令牌路径」下 pseudoconsole 连接全面失效**，与本项目 spawn 代码无关（Windows Terminal 正常是因其打包自带的 OpenConsole，不走 inbox conhost）。已加 `CREATE_NO_WINDOW` 防弹窗（attribute 失效时不再新建可见控制台），受影响机器上终端表现为打开成功但 `terminal_write` 返回空输出。诊断探针三件套保留于 `crates/harness-platform/examples/`（`gui_pty` 单端附着、`attach_matrix`/`attach_matrix_console` 矩阵扫组合，结果写系统临时目录）。
- **ConPTY 来源双路径（2026-11-22 根治落地）**：`PtyService::conpty_source()` 报告实际来源——优先侧载 `conpty.dll`（官方 NuGet `Microsoft.Windows.Console.ConPTY`，MIT；其内部启动**自带的 OpenConsole.exe** 而非系统内置 conhost，绕开上述平台回归），缺失时回退系统内置（inbox，CI 与正常机器零影响）。搜索顺序：exe 同目录 `binaries/conpty/`（桌面端 tauri resources）→ exe 同目录 → 当前工作目录 → PATH。dll 与 OpenConsole.exe 必须同目录同版本配对（conpty.dll 按相对路径找 OpenConsole）。构建脚本 `scripts/build-windows.ps1` 从 NuGet 固定版本下载提取（资产不入库）。**开发机实测：OpenConsole 路径下真实 PTY 集成测试 6/6 附着成功（marker 回流）——此前 GUI 宿主零输出的直接修复证据**；来源判定测试 `conpty_source_reflects_sideload_presence` 固化契约（dll 在场必须侧载）。端点安全软件对随包 OpenConsole.exe 的误报为社区已知分发知情项。
