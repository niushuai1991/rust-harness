# 内置浏览器集成计划

- 文档日期：2026-11-06（计划批准）
- 状态：已实施，真实模型端到端验证通过（2026-11-06）；桌面可视面板、CLI 调试子命令、多标签页不在本计划范围
- 范围：Agent 会话内的浏览器工具组（导航、快照、点击、输入、按键、截图、评估、关闭），系统 Edge/Chrome headless 内核与自研轻量 CDP 客户端。

## 背景与目标

Agent 目前仅有 `web_fetch`（reqwest 纯文本抓取），无 JS 渲染与交互能力，无法承担网页测试（验证自研前端、操作页面后断言结果）。本计划参考 ZCode（`D:\code\zcode`）的三层浏览器架构，引入内置浏览器能力：

- 统一命令契约（`BrowserCommand`），后端可替换；
- 第一版唯一后端为 headless CDP 后端（系统已装 Edge，Win11 自带；兼容 Chrome），不自动下载浏览器；
- 桌面可视面板、CLI 调试子命令、多标签页管理不在本计划范围，协议层为其留位。

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

## 实施决策与验证

- Ruling: Edge/Chrome（Windows）的 launcher 进程会转交真正的浏览器进程后立即退出（直接 spawn 版本子目录真身同样退出），stderr 的 `DevTools listening` 行不可依赖。就绪检测改为轮询 user-data-dir 下的 `DevToolsActivePort` 文件（首行为实际端口）；进程树清理改用 Job Object：spawn 后立即 `AssignProcessToJobObject`，真身继承 Job，实测关闭 Job 句柄后浏览器进程树全部终止。
- Ruling: 不自动下载浏览器；未在标准路径找到 Edge/Chrome 时返回 `BROWSER_NOT_FOUND` 并提示可用 `RUST_HARNESS_BROWSER_EXECUTABLE` 指定。
- TDD：单元测试 8 项（URL 白名单、候选路径、版本子目录 stub 解析、命令契约）先红后绿；工具链路 2 项（审批通过后触达注入的假后端、拒绝不触达）；集成测试 1 项经本地 HTTP 页面驱动真实 Edge headless 走完全链路（无浏览器环境打印原因跳过）。
- 全套验证：`cargo fmt --check`、`cargo clippy`（harness-browser 全目标零警告）、`cargo test --all-features`、`cargo build --all-features` 通过；分支 rebase 到 main（LSP 合并后）重新验证。
- 真实模型端到端（SQLite 快照独立数据目录 + 凭据管理器复用，glm-5.3-flash，`--approval auto`）：模型自主完成 navigate→snapshot→type(张三)→click→evaluate，读到"你好，张三"并报告，turn completed。
- 边界：CI 无浏览器环境时集成测试跳过（不失败）；桌面可视面板、CLI 调试子命令、多标签页管理未实现，协议层（命令契约 + 后端 trait）为其留位。

## 验收标准

- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 通过。（已完成）
- 本机存在 Edge 时，集成测试真实走通 navigate→snapshot→click→type→screenshot→evaluate。（已完成）
- 真实模型端到端抽查：会话内让模型打开本地测试页并交互验证，工具链路可用。（已完成）
