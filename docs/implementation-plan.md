# Rust Harness 实施计划

本文件落实用户于 2026-11-03 确认的设计。参考 deepseek-harness `ce7073d512` 与 OpenIDEA `00e717d8`。

## 全局约束

- Rust 1.98.1 / edition 2024，Tauri 2，React / TypeScript / Vite / Zustand，Windows 11 x64 首发。
- 独立 Rust 内置模块、WASM Component/WIT 插件、MCP；声明式 UI；不兼容 Cordis/npm 插件协议。
- SQLite WAL 事件为事实来源。模型输入可重建，副作用不自动重放，失败工具调用必须结算。
- 一切 Agent 文件/Shell操作针对托管工作副本；人工确认差异后由宿主同步原项目。
- 基线/备份/同步日志在宿主数据区，独占同步锁，外部修改冲突先行检测。
- Windows 受限令牌+Low IL+能力SID+ACL仅施加托管根和私有temp；Job Object清理进程树；失败拒绝启动。
- WASM独立Store、256MiB、5e9 fuel、hook5s/tool60s；权限取声明、授权、工作区策略交集。
- NSIS简中/英文，标准WebView2 bootstrapper及离线installer配置；升级保留数据。安装权限按2026-11-04用户反馈修订：0.1.1起使用perMachine，在开始安装前请求UAC，以支持受保护目录。
- 不改用户全局配置，不触碰参考项目。密钥用Windows凭据管理器，遥测关闭。

## Task 1: 共享接口与事件存储
类型化IPC数据、SQLite迁移/WAL、会话事件、恢复、搜索、导入导出、设置、附件/spill。验证事件排序、重启恢复、未来schema拒绝、失败输入。

## Task 2: 平台、工作副本与Windows隔离
独立Git/普通文件副本、路径检查、宿主基线、差异预览/冲突/备份应用、进程树和系统沙箱、凭据。验证越界、链接、冲突、真实进程写入边界。

## Task 3: 插件体系
manifest/API版本/依赖、目录/ZIP安全安装、WIT+Wasmtime、声明式贡献、权限和生命周期、MCP stdio/HTTP。验证崩溃、超限、授权、安装攻击和在途停用。

## Task 4: Agent循环与能力
模型SSE、工具流水线、审批、取消、工作区指令与skills、文件/Shell、子代理/jobs/todo/plan/goal、压缩、调度、预设、CLI。验证录制流、完整工具循环、拒绝/取消/恢复。

## Task 5: 桌面、安装与扩展交付
Tauri IPC/Channel/Specta、聊天/工作区/模型/审批/插件/同步UI、托盘、NSIS标准/离线、CI、更新边界。P3: SSH/LSP/web/document、headless JSON-RPC/SDK/ACP、市场签名和脚本workflow。

## 阶段验收

|阶段|要求|
|---|---|
|P0|实际构建、空壳NSIS、工作副本/Windows沙箱/WASM可行性|
|P1|读取→副本修改→验证→人工同步→重启继续；模型、插件、MCP、桌面与安装|
|P2|子代理、后台任务、PTY、计划/目标、压缩、搜索/导出、调度和预设|
|P3|远端执行、LSP/web/doc、协议接口、市场、签名更新与可选脚本运行时|

浏览器自动化/CUA/语音/实时协作属后续可选能力。每项状态以 capability-matrix.md 为准，不以接口预留充当完成。
