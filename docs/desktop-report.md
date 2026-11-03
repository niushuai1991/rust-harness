# 桌面交付报告

## 实现

- React 19 / TypeScript / Vite / Zustand 中文桌面界面，工作区与会话切换、聊天增量、独立 reasoning 折叠区、工具调用/结果卡片、停止生成、错误和空状态。
- Tauri `invoke` 顶层 camelCase 参数，`session_send` 使用原生 `Channel<LiveEvent>`；浏览器预览明确提示需要桌面宿主，无虚构模型响应，无 localStorage 业务存储。
- 消息以宿主持久化事件为事实来源；增量按会话和请求区分，重复片段去重，持久化序号按 BigInt 排序。模型文本仅通过 React 文本节点呈现，不执行 HTML。
- 模型编辑、保存、测试；密钥密码输入且仅发送不读取，保存后清空。插件本地目录/ZIP 路径导入、显式 grants、启停和移除确认。
- 审批轮询显示真实待批准操作，批准/拒绝携带请求 id；差异逐文件预览、冲突禁选、勾选、再次确认应用、同步后刷新。
- Tauri 2 桌面壳、单实例、窗口状态、对话框、opener、updater 插件初始化；独立 `desktop.rs` 实现托盘显示/退出及关闭隐藏。业务 commands 和该 setup 由主实现者接线。
- CSP 禁止远程脚本、iframe、object；能力仅开放 core 生命周期和 open/save 对话框，不开放任意 fs/shell 或 opener IPC。无伪造更新公钥、无更新 endpoint。
- 应用标识 `dev.rustharness.desktop`，真实多尺寸 ICO 与 PNG 图标，NSIS currentUser、SimpChinese/English、lzma、内嵌 WebView2 bootstrapper；独立离线安装器 override。

## 本地验证（2026-11-03）

- `npm install --no-audit --no-fund` 成功，生成 `package-lock.json`，未改全局 Node 配置。
- `npm test`：6 项通过，覆盖会话切换、真实发送/增量/最终回读、错误展示、审批、差异选择与二次确认、重复事件与大整数排序。
- `npm run build`：TypeScript 校验和生产构建通过，JS 约 254 KB / gzip 79 KB。
- 图标 ICO 头及多尺寸图像已生成核验。
- `npx tauri info` 配置解析通过，确认本机 Rust 1.98.1、MSVC 和现有 WebView2 156。复用系统已有 Edge headless 渲染 1440×940 界面并查看截图，未安装额外浏览器，截图位于 `.cache/desktop-preview.png`（本地临时验证产物）。
- 未下载浏览器、WebView2 完整离线包；本报告不将 NSIS 安装或 Windows 进程 smoke 标为已验证。Rust 全工作区检查和实际安装构建由集成阶段执行。

## 开发和打包

```powershell
npm ci --no-audit --no-fund
npm run tauri dev
npm run desktop:build
# 可选：生成包含完整 WebView2 的较大离线安装包（需要额外下载）
npm run desktop:offline
# 标准构建连同前端验证
./scripts/build-windows.ps1
# 已构建二进制启动 / 第二实例退出验证
./scripts/smoke-windows.ps1
```

打包产物位于 `target/release/bundle/nsis`。Node 只参与开发和构建，安装后的桌面应用运行时不依赖 Node。CI 默认构建标准安装包，workflow_dispatch 可显式选择离线包。

## 数据保留策略

安装目录只保存应用文件。SQLite、工作副本、宿主基线、备份与同步日志必须存储在宿主管理的数据目录中，不写入应用安装目录。普通覆盖安装/升级不得清理数据目录。卸载默认保留用户数据与 Windows 凭据；用户显式清除数据时应先导出会话、确认待同步更改，退出应用后再单独清理对应应用数据和凭据。安装脚本不递归删除项目目录、数据目录或系统 WebView2。

## 边界

`frontend/types.ts` 当前为人工对齐 Rust contracts 的类型镜像，未声称是 Specta 生成。`tauri-specta` 为宿主生成类型预留依赖；需要集成方验证/导出最终 commands。实际 IPC 的文件系统与审批安全性由 Rust 宿主保证。前端 DOM 测试不代替真实桌面、安全隔离、凭据或安装验收。

## 独立审阅修复（2026-11-03）

- 新建会话绑定请求发起时的工作区和 generation，并核对当前工作区与响应归属；工作区 A 的迟到响应不会污染已切换的工作区 B，也不会把后续发送目标改回 A。回归测试先在旧实现复现失败，再验证修复后仍发送到 B 会话。
- `workspace_apply` 除路径外传递用户实际审阅并选中的完整 `FileChange[] reviewed`，包含三种 hash、原/新内容和冲突字段，由宿主执行审阅快照校验。刷新差异会清除旧确认状态；宿主拒绝过期快照时显示错误，不显示同步成功。
- 插件对话框新增 MCP 连接区：`mcp_list` 展示实际活动连接；用户显式提交 `streamableHttp` 或 `stdio` 配置进行连接，显式按 id 断开；不自动连接或重连。stdio 参数按 JSON 字符串数组校验，并提示长期进程占用工作副本、同步前需断开。
- MCP 回归覆盖 HTTP 连接/断开、stdio 参数错误与完整配置、握手失败；连同会话切换与快照回归，最终共 12 项前端测试。
