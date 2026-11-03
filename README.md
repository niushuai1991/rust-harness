# Rust Harness

Windows 11 上运行的本地开发协作工具：Rust / Tauri 2 宿主，React 19 界面，SQLite 事件存储，独立托管工作副本。模型与工具修改工作副本，用户审阅差异后同步原项目。

## 模型供应商

内置 DeepSeek 与智谱 Coding Plan 两个供应商，每家内部支持 Chat Completions、Responses、Anthropic 三种协议。内置端点只读，需要改地址时复制为自定义供应商。协议间共享该供应商的 API Key 与代理；保存模型后可进行少量实际生成连接测试。详见[三协议交付说明](docs/multi-protocol-delivery.md)。

## 开发

需要 Rust stable（当前验证版本 1.98.1）、MSVC C++ Build Tools、Windows SDK、WebView2，以及 Node.js 22。Node 仅用于开发和打包，最终应用运行时不依赖 Node。

项目唯一依赖锁为 `package-lock.json`，CI 和开发均使用 npm；不要生成第二套 pnpm/yarn 锁文件。

```powershell
npm ci --no-audit --no-fund
npm run tauri dev
```

`npm run dev` 只启动浏览器界面预览，需要桌面宿主才能使用工作区、会话、模型与插件。界面预览不会伪造模型响应。

## 验证

```powershell
npm test
npm run build
npm run sdk:check
cargo test --workspace
cargo check -p rust-harness-desktop
python -m unittest discover -s sdk/python -p 'test_*.py' -v
```

## Windows 安装包

```powershell
# 标准：系统级安装（开始时请求UAC），内嵌 WebView2 bootstrapper
./scripts/build-windows.ps1
# 仅打包（先自行完成安装依赖与测试）
npm run desktop:build
# 可选离线 WebView2 完整安装包：额外下载量较大
npm run desktop:offline
# 验证已构建桌面进程启动、第二实例退出
./scripts/smoke-windows.ps1
```

NSIS 产物位于 `target/release/bundle/nsis`。从0.1.1起采用系统级安装，安装开始时由Windows请求管理员权限（UAC），可安装到Program Files或其他受保护目录，卸载信息写入HKLM。应用本身仍以普通用户权限启动。支持简体中文与英文、LZMA压缩；标准包缺少WebView2时需要网络，离线包通过 `src-tauri/tauri.offline.conf.json` 启用。构建脚本及CI使用 `scripts/verify-installer.py` 读取生成EXE的PE清单，确认其为 `requireAdministrator`。仓库不带伪造的更新endpoint或签名公钥。

关闭主窗口会保留后台会话并隐藏到托盘，托盘菜单可以重新打开或退出应用。

## 数据与权限

应用文件与用户数据分开存储。覆盖安装和升级保留会话、托管副本、基线、同步日志与备份；卸载默认保留用户数据和凭据。API 密钥仅写入 Windows 凭据管理器，模型列表不会读取或回显密钥。插件仅在声明、授权与工作区策略交集内运行。请先导出会话、处理待同步更改，再显式清理应用数据。

实现与验收边界见 [实施计划](docs/implementation-plan.md)、[桌面交付报告](docs/desktop-report.md)。

## Headless 与 SDK

```powershell
cargo run -p harness-cli -- --help
cargo run -p harness-cli -- --data-dir .cache/cli-data rpc
cargo run -p harness-cli -- --data-dir .cache/cli-data call workspace.list '{}'
```

标准输入/输出为逐行 JSON-RPC，支持工作区、会话、模型、审批、插件和 MCP；`session.event` 推送日志和模型增量。同步方法必须传递完整 `reviewed` 差异快照。Python 标准库客户端位于 `sdk/python/rust_harness.py`，TypeScript 客户端位于 `sdk/typescript/client.ts`。同一数据目录只允许一个宿主，桌面运行时请给 CLI 指定独立目录。

当前交付为开发预览。阶段完成度和未实现项目见 [能力矩阵](docs/capability-matrix.md)。尚未实现 ACP、SSH、LSP、真实 PTY、签名插件市场及脚本工作流。0.1.2标准NSIS约12.71MiB，离线WebView2包约218.71MiB，均已实际生成；尚未做干净虚拟机安装验收。正式更新需提供公钥、端点并启用 `signed-updates` 功能，默认不会初始化未配置的更新服务。
