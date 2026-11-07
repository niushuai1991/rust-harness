# P1 验收报告

日期：2026-11-04。状态：P1 验收通过。

验收依据：[P1 验收与缺口补齐](specs/p1-acceptance-spec.md)。本报告后续以实际执行结果更新，历史版本记录不代表本次验收证据。

## 已取得的结果

- 两个供应商的六条真实连接测试已通过，使用 Windows 凭据管理器中已配置的凭据，未导出密钥。DeepSeek 使用 deepseek-flash，智谱使用 glm-5.3-flash。
- 首次智谱 Responses 在 128 token 预算下返回截断；新增本地回归先观察失败，再将连接测试预算调整为 512 后通过，六路真实复验通过。
- 两个供应商、三种协议的真实 Agent 链路均已通过，共 6 条链路、24 次模型请求和 18 次真实工具结果。每条链路均完成读取、审批后副本写入、固定 Shell 验证、审阅同步和独立 CLI 重启后的历史检查。
- Rust workspace 全量回归、全工作区严格 Clippy、前端生产构建、34 项前端交互测试和 TypeScript SDK 类型检查均已通过。附件、声明式插件界面、受控 HTTP、MCP fixture、工作副本同步和安全卸载均有专项回归。
- 0.1.4 单一 NSIS 安装包已构建，使用系统 WebView2 的 skip 模式。SHA-256 为 `A17E3E6B3CB7F94E0F4C8E5F1221143EC030A9CA3FBAB887B012E4166ED009C0`。

本地证据：`.cache/p1-live-models-512/live-model-results.json`、`.cache/p1-live-models-512.log`、`.cache/p1-live-agent-20261104-a/live-agent-results.json`、`.cache/p1-rust-final.log`、`.cache/p1-clippy-final.log`。

## 实机安装与桌面验收

Windows 11 隔离 guest `07e3c230fd4f4211bcc1e80d3379ffbf` 完成通过：0.1.2 安装、覆盖升级至 0.1.4、三处用户数据保留、原生输入与发送、写入拒绝和批准、Shell 审批、差异同步、关闭驻托盘、单实例唤回及重启继续会话均通过。默认卸载保留数据；显式勾选“删除应用程序数据”后，三个固定应用数据根均被删除，外部项目、独立运行时目录和 junction 目标均被保留。

首轮 guest 发现 Wry 只从窗口配置读取 WebView2 附加参数，因此新增了仅对 `P1_SANDBOX_GUEST=1`、`WDAGUtilityAccount` 和有效端口同时成立的受限调试分支。后续诊断修正了 NSIS 的语言选择页和该 guest 将复选框暴露为 `Pane` 的 UI Automation 差异；最终验收以实际数据根和外部目标状态作为结论。

## 当前验收边界

不终止或修改宿主正在运行的用户应用，不使用用户项目作为写入测试对象。真实模型只访问 `.cache` 内合成工作副本。管理员 guest 不代替标准用户 UAC 安全桌面交互验证；安装包的 perMachine/UAC 清单和实际受保护目录安装已在 guest 中验证。
