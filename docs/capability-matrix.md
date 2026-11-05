# 能力状态（2026-11-04）

状态说明：已实现表示存在实际执行代码；已验证另需对应测试或实际构建证据。不以计划或接口占位替代功能。

|能力|实际状态|验证|
|---|---|---|
|Rust/Tauri桌面、中文聊天与设置|已实现并验证|0.1.4前端34项测试与生产构建通过；Windows 11 guest 中完成输入、审批、同步、托盘、单实例与重启继续会话|
|DeepSeek/智谱双供应商与Chat/Responses/Anthropic|已实现并接线|六条固定路由、三协议本地HTTP/SSE工具审批及重启回放、复制兼容与凭据隔离；两供应商六条真实连接及 Agent 工具链路已通过|
|SQLite WAL、游标、恢复、FTS、JSONL、附件/spill|已实现并验证|持久顺序、未知结果结算、导入事务；schema2备份迁移、回滚和重试；附件导入、会话隔离、分块读取、下载与重启回归|
|原项目/执行副本分离与人工同步|已实现|冲突、预览hash、ignore、链接、Git独立副本回归|
|Windows受限token/Low IL/ACL/Job/stdio|已实现|真实Windows内部允许、外部拒绝、取消子树和split生命周期测试|
|WASM/WIT、权限、ZIP、hook、KV、声明式界面|已实现并接线|真实component超限/取消/停用与安装攻击；设置、审批命令、受控HTTP、主题和安全卡片回归|
|MCP HTTP/stdio、资源、取消|已实现并接线|官方SDK本地HTTP和stdio fixtures；自动后台重连未实现|
|后台Shell、子代理spawn/fork/send/wait/interrupt|已实现并接线|协作与后台任务测试|
|Todo/Plan/Goal|事件与模型工具已实现|尚无独立管理面板|
|调度、会话压缩|应用存活期间单次调度、显式压缩服务已实现|missed不补跑、缺模型failed和原日志保留测试|
|headless/stdio JSON-RPC|已实现|真实子进程协议错误恢复和请求测试通过|
|Python/TypeScript SDK|源码已实现|Python SDK真实子进程2项通过；TypeScript SDK独立类型检查通过，尚未独立发布|
|NSIS安装包|0.1.4单包11.17MiB，使用系统已有WebView2|Windows 11 guest 中完成 0.1.2 安装、0.1.4 覆盖升级、默认卸载保留数据及显式清理三固定根；外部目标保留|
|NSIS离线包|仅保留0.1.2及以前的历史产物，后续不再提供|后续仅构建系统已有WebView2的单一NSIS包（skip模式）|
|发布签名与自动更新|Tauri插件接入，信任配置未启用|没有签名证书、更新公钥或发布端点，未发布|
|LSP只读五件套（诊断/悬停/定义/引用/符号）|已实现并接线|mock server 11项与runtime流程6项测试；真实rust-analyzer经Windows沙箱冒烟通过（E0308诊断与hover到位），测试为`#[ignore]`仅显式运行，见[LSP计划](specs/lsp-plan.md)|
|PTY/SSH执行世界/搜索provider/Office预览|未实现|不能计为P2/P3完成|
|ACP/配置预设分层/脚本工作流/签名插件市场|未实现|不能计为P2/P3完成|

数据目录为宿主独占，桌面打开时headless需使用独立数据目录，避免另一个进程执行恢复时破坏活动会话。WASM采用每次调用独立Store，activate→execute/hook→deactivate，KV成功后提交；不承诺长期驻留实例状态。

0.1.2 完整验证：75 项 Rust、23 项前端、2 项 Python SDK；详见[三协议交付说明](multi-protocol-delivery.md)。

0.1.3修复首次选择空工作区时输入框禁用，新增5项回归并通过前端全部28项测试；单安装包及边界见[交付记录](delivery.md)。
