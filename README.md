# Rust Harness

> Windows 上的本地 AI 编程协作宿主：模型在隔离的工作副本里修改代码，你审阅差异后同步回原项目。

[![Release](https://img.shields.io/github/v/release/niushuai1991/rust-harness)](https://github.com/niushuai1991/rust-harness/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%2011-blue)](https://github.com/niushuai1991/rust-harness/releases)

Rust / Tauri 2 宿主，React 19 界面，SQLite 事件存储。所有模型改动只发生在独立托管的执行副本中，原项目始终由你掌控。

## ✨ 功能特性

- **隔离协作**：原项目与执行副本分离，模型产物经差异审阅、人工确认后同步；冲突预览、ignore 规则与基线管理齐备。
- **多供应商多协议**：内置 DeepSeek 与智谱 Coding Plan，各支持 Chat Completions、Responses、Anthropic 三种协议；协议间共享 API Key 与代理配置。
- **完整 Agent 工具链**：文件编辑、后台 Shell、PTY 沙箱终端（自研 ConPTY + 受限 token）、内置浏览器（Edge/Chrome CDP）、LSP 只读工具（诊断/悬停/定义/引用/符号）、子代理 spawn/fork。
- **多种接入形态**：桌面端、`harness` CLI（run/chat/diff/apply/models/sessions）、ACP v1 agent 端（供编辑器客户端接入）、JSON-RPC over stdio、Python / TypeScript SDK。
- **插件与扩展**：WASM(WIT) 组件权限沙箱、MCP（HTTP/stdio）、命名配置预设（模型/effort/审批策略组合）。
- **安全模型**：Windows 受限 token 与 Job Object 沙箱、API Key 仅写入凭据管理器、工作区策略与审批策略分层管控。
- **协作可视化**：Todo/Plan/Goal 卡片、后台任务面板、子代理运行状态、跨会话计划与目标总览、托盘常驻。

## 🚀 快速开始

### 安装

从 [Releases](https://github.com/niushuai1991/rust-harness/releases) 下载最新 NSIS 安装包：系统级安装（安装时请求管理员权限），需要系统已有 WebView2，支持简体中文与英文界面。覆盖安装和升级保留会话、托管副本等用户数据；卸载默认保留用户数据和凭据。

也可以从源码构建，见[开发](#-开发)一节。

### 首次使用

1. 打开应用，选择一个本地项目目录作为工作区；
2. 在「模型设置」中配置供应商 API Key（仅写入 Windows 凭据管理器，界面不会回显）；
3. 发起会话——模型在隔离副本中工作，你在差异视图中审阅并同步回原项目。

打开工作区会自动恢复上次会话；没有历史会话时自动创建首个会话。关闭主窗口会保留后台会话并隐藏到托盘。

## 📖 文档

| 文档 | 内容 |
|---|---|
| [规格文档索引](docs/spec-index.md) | 全部设计规范与实施计划入口 |
| [能力状态矩阵](docs/capability-matrix.md) | 各能力的实现与验证状态 |
| [总体实施计划](docs/specs/implementation-plan.md) | 全局架构、模块职责与阶段规划 |
| [三协议交付说明](docs/multi-protocol-delivery.md) | 供应商与协议配置、连接测试细节 |

### CLI 与 SDK

```powershell
# CLI
cargo run -p harness-cli -- --help
cargo run -p harness-cli -- --data-dir .cache/cli-data call workspace.list '{}'

# JSON-RPC over stdio（headless）
cargo run -p harness-cli -- --data-dir .cache/cli-data rpc

# ACP agent 端
cargo run -p harness-cli -- acp
```

标准输入/输出为逐行 JSON-RPC，支持工作区、会话、模型、审批、插件和 MCP；`session.event` 推送日志和模型增量。Python 客户端位于 `sdk/python/rust_harness.py`，TypeScript 客户端位于 `sdk/typescript/client.ts`。

同一数据目录只允许一个宿主进程；桌面运行时请给 CLI 指定独立 `--data-dir`。

## 🏗 架构

- 桌面宿主 Tauri 2 + React 19；SQLite（WAL）事件存储为唯一事实来源，支持游标、恢复、全文检索与 JSONL 导出。
- 执行世界基于 Windows 受限 token、Low IL 与 Job Object 的本地沙箱；PTY、浏览器（CDP）与 LSP 由独立模块承载。
- crate 划分：`harness-contracts` / `harness-storage` / `harness-orchestration` / `harness-runtime` / `harness-platform` / `harness-extensions` / `harness-browser` / `harness-cli`。

## 🛠 开发

环境要求：Rust stable（经 `rust-toolchain.toml` 指定）、MSVC C++ Build Tools、Windows SDK、WebView2、Node.js 22。Node 仅用于开发和打包，最终应用运行时不依赖 Node。

项目唯一依赖锁为 `package-lock.json`，CI 和开发均使用 npm；不要生成第二套 pnpm/yarn 锁文件。

```powershell
npm ci --no-audit --no-fund
npm run tauri dev   # 桌面端
npm run dev         # 仅浏览器界面预览（无桌面宿主能力）
```

测试与验证：

```powershell
npm test
npm run build
npm run sdk:check
cargo test --workspace
cargo check -p rust-harness-desktop
python -m unittest discover -s sdk/python -p 'test_*.py' -v
```

构建 Windows 安装包：

```powershell
./scripts/build-windows.ps1   # 系统级 NSIS 安装包
./scripts/smoke-windows.ps1   # 冒烟：桌面进程启动、第二实例退出
```

产物位于 `target/release/bundle/nsis`。

## 📍 当前状态

项目处于开发预览阶段，迭代较快。已完成能力的实现与验证边界见[能力状态矩阵](docs/capability-matrix.md)；尚未实现 SSH 远端执行世界、搜索 provider、Office 预览、脚本工作流、签名插件市场、发布签名与自动更新。

## 🤝 贡献

欢迎提交 Issue 与 PR。涉及功能、架构或交付要求时，请先查阅[规格文档索引](docs/spec-index.md)及相关规范，并在 PR 中附改动摘要与验证结果。

## 📄 许可

本项目基于 [Apache-2.0](LICENSE) 发布；第三方依赖各自保留其原有许可。
