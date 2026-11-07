# 内置浏览器集成规范

- 日期：2026-11-06（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：Agent 会话内的浏览器工具组（导航、快照、点击、输入、按键、截图、评估、关闭），系统 Edge/Chrome headless 内核与自研轻量 CDP 客户端；桌面可视面板、CLI 调试子命令、多标签页不在范围，协议层为其留位。

## 背景与目标

Agent 仅靠 `web_fetch`（reqwest 纯文本抓取）无 JS 渲染与交互能力，无法承担网页测试（验证自研前端、操作页面后断言结果）。本规范参考 ZCode（`D:\code\zcode`）的三层浏览器架构，约定内置浏览器能力：

- 统一命令契约（`BrowserCommand`），后端可替换；
- 第一版唯一后端为 headless CDP 后端（系统已装 Edge，Win11 自带；兼容 Chrome），不自动下载浏览器。

## 架构

```
Agent 会话
  └─ browser_* 工具组（harness_browser::install 注册，仿 harness-orchestration）
      └─ BrowserCommand / CommandOutcome（serde 契约）
          └─ BrowserBackend trait（execute/close，工厂注入便于测试）
              └─ CdpBackend：进程管理（launcher）+ CDP WebSocket（cdp）+ 元素收集（snapshot）
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
| `browser_navigate` | `url`，可选 `action: back\|reload` | 打开 URL，仅允许 http/https/about:blank |
| `browser_snapshot` | 无 | 返回可交互元素清单（ref/role/tag/name/value），操作元素前必须先快照 |
| `browser_click` | `ref` | 重定位元素并派发真实鼠标事件，返回新快照 |
| `browser_type` | `ref`、`text`、可选 `submit` | 聚焦后 `Input.insertText`，可追加回车 |
| `browser_press` | `key` | 常用键（Enter/Tab/Escape/方向键等）虚拟键码映射 |
| `browser_screenshot` | 可选 `fullPage` | PNG 保存到 `<数据目录>/browser/screenshots`，返回路径与尺寸 |
| `browser_evaluate` | `expression` | 只读评估页面状态，`Runtime.evaluate` returnByValue |
| `browser_close` | 无 | 关闭会话并终止浏览器进程 |

安全边界（对齐 ZCode 结论）：页面内容视为不可信，仅用于定位；导航 scheme 白名单；工具默认走审批（`ExternalTool::requires_approval` 默认 true）；浏览器使用独立临时 user-data-dir，不携带用户配置与登录态。

## 接入点

- 根 `Cargo.toml`：`harness-browser` 加入 members/default-members 与 workspace 依赖；新增 `tokio-tungstenite`。
- `crates/harness-cli/src/main.rs` 与 `src-tauri/src/lib.rs`：在 `orchestration::install` 之后追加 `harness_browser::install(runtime.clone())`。
- `crates/harness-platform/src/workspace.rs`：为 `WorkspaceManager` 增加 `data_dir()` 只读访问器，用于确定截图保存目录。
