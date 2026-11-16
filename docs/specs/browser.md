# 内置浏览器集成规范

- 日期：2026-11-06（计划批准）；2026-11-16（命令契约新增 forward、后端复用面公开；桌面可视后端落地）修订
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：Agent 会话内的浏览器工具组（导航、快照、点击、输入、按键、截图、评估、关闭），系统 Edge/Chrome headless 内核与自研轻量 CDP 客户端；桌面端另含 WebView2 可视后端（内嵌浏览器人机共用一个实例）；CLI 调试子命令、多标签页不在范围，协议层为其留位。

## 背景与目标

Agent 仅靠 `web_fetch`（reqwest 纯文本抓取）无 JS 渲染与交互能力，无法承担网页测试（验证自研前端、操作页面后断言结果）。本规范约定内置浏览器能力采用分层架构：

- 统一命令契约（`BrowserCommand`），后端可替换；
- 第一版唯一后端为 headless CDP 后端（系统已装 Edge，Win11 自带；兼容 Chrome），不自动下载浏览器。

## 架构

```
Agent 会话
  └─ browser_* 工具组（harness_browser::install 注册，仿 harness-orchestration）
      └─ BrowserCommand / CommandOutcome（serde 契约）
          └─ BrowserBackend trait（execute/close，工厂注入便于测试）
              ├─ CdpBackend（CLI / 非 Windows）：进程管理（launcher）+ CDP WebSocket（cdp）+ 元素收集（snapshot）
              └─ WebView2Backend（桌面 Windows，src-tauri/browser_backend.rs）：进程内 CDP 驱动「网页」面板
```

新 crate `crates/harness-browser`：

| 模块 | 职责 |
|---|---|
| `command.rs` | `BrowserCommand`、`CommandOutcome`、`SnapshotView`、`ElementInfo`、`ScreenshotInfo`、URL 白名单 |
| `launcher.rs` | 可执行文件解析（显式 > 环境变量 > Edge/Chrome 标准安装路径，命中后优先版本子目录真身）；`--headless=new --remote-debugging-port=0` 启动；轮询 user-data-dir 下 `DevToolsActivePort` 等待就绪；Job Object（KILL_ON_JOB_CLOSE）保证进程树退出清理 |
| `cdp.rs` | CDP WebSocket 客户端：请求 id 匹配 + 事件广播 + 连接/调用超时；封装 Page/Runtime/Input 域调用 |
| `snapshot.rs` | 可交互元素收集脚本与重定位脚本；ref→CSS 定位映射保存在 Rust 侧，不落入页面 `globalThis` |
| `backend.rs` | `BrowserBackend` trait + `CdpBackend`；导航等待 load、点击重定位、截图落盘 |
| `lib.rs` | `install(runtime)` / `install_with(runtime, factory)`；懒启动、串行执行、空闲超时回收 |

### 平台行为与关键决策

- Edge/Chrome（Windows）的 launcher 进程（含版本子目录里的真身）spawn 后会立即退出，把真正的浏览器进程转交出去：stderr 的 `DevTools listening` 行不可依赖（anonymous pipe 随 launcher 关闭而 EOF），`child.kill`/`kill_on_drop` 杀不到真身。因此就绪检测轮询 user-data-dir 下的 `DevToolsActivePort` 文件（首行为实际端口）；进程树清理用 Job Object：spawn 后立即 `AssignProcessToJobObject`（KILL_ON_JOB_CLOSE），真身继承 Job，关闭 Job 句柄可终止整棵进程树。
- 不自动下载浏览器；未在标准路径找到 Edge/Chrome 时返回 `BROWSER_NOT_FOUND` 并提示可用 `RUST_HARNESS_BROWSER_EXECUTABLE` 环境变量指定。
- CI/无浏览器环境时集成测试跳过（不失败）。

## 工具面

| 工具 | 参数 | 说明 |
|---|---|---|
| `browser_navigate` | `url`，可选 `action: back\|forward\|reload` | 打开 URL，仅允许 http/https/about:blank |
| `browser_snapshot` | 无 | 返回可交互元素清单（ref/role/tag/name/value），操作元素前必须先快照 |
| `browser_click` | `ref` | 重定位元素并派发真实鼠标事件，返回新快照 |
| `browser_type` | `ref`、`text`、可选 `submit` | 聚焦后 `Input.insertText`，可追加回车 |
| `browser_press` | `key` | 常用键（Enter/Tab/Escape/方向键等）虚拟键码映射 |
| `browser_screenshot` | 可选 `fullPage` | PNG 保存到 `<数据目录>/browser/screenshots`，返回路径与尺寸 |
| `browser_evaluate` | `expression` | 只读评估页面状态，`Runtime.evaluate` returnByValue |
| `browser_close` | 无 | 关闭浏览器会话（headless 后端同时终止浏览器进程） |

安全边界：页面内容视为不可信，仅用于定位；导航 scheme 白名单；工具默认走审批（`ExternalTool::requires_approval` 默认 true）；headless 后端使用独立临时 user-data-dir，不携带用户配置与登录态。

## 桌面可视后端（WebView2，2026-11-16 修订）

桌面端经 `install_with(runtime, WebView2Factory)` 注入可视后端：右侧面板「网页」视图的 WebView2 既是用户浏览的界面，也是模型 `browser_*` 工具操作的浏览器——人机共用同一个实例，模型操作全程可见。

- 执行路径：工具调用（tokio 任务）→ 主线程 `with_webview` → 进程内 CDP（`ICoreWebView2::CallDevToolsProtocolMethod`）→ oneshot 回传；导航等待 `NavigationCompleted`（15s 超时不视为失败），命令语义（`errorText`、导航后 ref 失效、点击后 800ms 导航窗口、截图落盘）与 headless 后端同构。导航统一走 CDP（`Page.navigate/navigateToHistoryEntry/reload`）；COM 的 `CanGoBack/CanGoForward/HistoryChanged` 仅用于工具栏禁用态。
- 生命周期：面板 webview 持久、归 UI 层所有——面板关闭仅隐藏不销毁；`browser_close` 与空闲回收只清 ref 映射，不销毁页面。webview 缺失时后端以 `about:blank`（或目标 URL）创建，并发事件自动弹出「网页」视图。
- 面板工具栏：后退/前进/刷新（`web_panel_history` 命令与模型命令共用同一后端路径，ref 映射一致失效）；地址栏实时跟随实际导航（`NavigationCompleted` 上报当前 URL）。`web_panel_open` 幂等：webview 已存在时不重建、目标与当前页一致时不重复导航。
- 面板设置：禁用右键菜单与浏览器快捷键（同主窗口处理），避免 F5 等绕过 ref 失效逻辑；ESC 关闭面板语义不变。
- 安全边界差异：可视后端与主窗口共用 WebView2 user data folder，用户在内嵌浏览器登录的站点登录态对 `browser_evaluate`/`browser_click` 等工具可见——与 headless 的临时目录隔离不同，以 browser_* 工具逐次审批把守，此为已知并接受的边界。
- 并发与共享：browser_* 命令经 `ManagerState` 全局串行；面板工具栏动作与模型命令可并行，以页面最终状态为准；多会话共享同一浏览器实例（与 headless 行为一致）。审批浮层打开期间 webview 隐藏，模型在审批通过后才实际操作。
- 降级策略：非 Windows（无 WebView2）回落 headless 后端；WebView2 创建失败直接报错返回给模型，不静默降级。

## 接入点

- 根 `Cargo.toml`：`harness-browser` 加入 members/default-members 与 workspace 依赖；新增 `tokio-tungstenite`。
- `crates/harness-cli/src/main.rs`：在 `orchestration::install` 之后追加 `harness_browser::install(runtime.clone())`（headless）。
- `src-tauri/src/lib.rs`：Windows 以 `install_with(runtime, WebView2Factory)` 注入可视后端并托管桥接状态，非 Windows 保持 `install`；`src-tauri/src/browser_backend.rs` 为后端实现（桥、命令执行、面板事件监听），`src-tauri/src/web_panel.rs` 提供幂等的面板 webview 创建与 `web_panel_history` 命令。
- `crates/harness-platform/src/workspace.rs`：为 `WorkspaceManager` 增加 `data_dir()` 只读访问器，用于确定截图保存目录。
