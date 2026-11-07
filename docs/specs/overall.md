# Rust Harness 总体规范

- 日期：2026-11-03（设计确认，原文依据）
- 状态：现行规范；各阶段能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：全局架构、模块职责、安全不变量、交付阶段划分与阶段验收要求。

设计来源参考 deepseek-harness `ce7073d512` 与 OpenIDEA `00e717d8`。

## 全局约束

- Rust 1.98.1 / edition 2024，Tauri 2，React / TypeScript / Vite / Zustand，Windows 11 x64 首发。
- 独立 Rust 内置模块、WASM Component/WIT 插件、MCP；声明式 UI；不兼容 Cordis/npm 插件协议。
- SQLite WAL 事件为事实来源。模型输入可重建，副作用不自动重放，失败工具调用必须结算。
- 一切 Agent 文件/Shell操作针对托管工作副本；人工确认差异后由宿主同步原项目。
- 基线/备份/同步日志在宿主数据区，独占同步锁，外部修改冲突先行检测。
- Windows 受限令牌+Low IL+能力SID+ACL仅施加托管根和私有temp；Job Object清理进程树；失败拒绝启动。
- WASM独立Store、256MiB、5e9 fuel、hook5s/tool60s；权限取声明、授权、工作区策略交集。
- NSIS简中/英文，后续版本仅提供单一安装包，默认系统已安装WebView2，使用skip模式，不内嵌、下载或安装运行时；升级保留数据。安装权限按2026-11-04用户反馈修订：0.1.1起使用perMachine，在开始安装前请求UAC，以支持受保护目录。
- 不改用户全局配置，不触碰参考项目。密钥用Windows凭据管理器，遥测关闭。

## 阶段验收

|阶段|要求|
|---|---|
|P0|实际构建、空壳NSIS、工作副本/Windows沙箱/WASM可行性|
|P1|读取→副本修改→验证→人工同步→重启继续；模型、插件、MCP、桌面与安装|
|P2|子代理、后台任务、PTY、计划/目标、压缩、搜索/导出、调度和预设|
|P3|远端执行、LSP/web/doc、协议接口、市场、签名更新与可选脚本运行时|

浏览器自动化/CUA/语音/实时协作属后续可选能力。每项状态以 [能力状态矩阵](../capability-matrix.md) 为准，不以接口预留充当完成。
