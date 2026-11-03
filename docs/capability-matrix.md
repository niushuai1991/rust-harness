# 能力状态（2026-11-04）

状态说明：已实现表示存在实际执行代码；已验证另需对应测试或实际构建证据。不以计划或接口占位替代功能。

|能力|实际状态|验证|
|---|---|---|
|Rust/Tauri桌面、中文聊天与设置|已实现并接线|前端12项测试、生产构建、发布版启动及单实例冒烟通过|
|DeepSeek/兼容SSE、推理和tool-call分片|已实现|分片UTF-8、工具参数与断流回归；未使用真实付费API|
|SQLite WAL、游标、恢复、FTS、JSONL、附件/spill|已实现|持久顺序、未知结果结算、导入事务回归|
|原项目/执行副本分离与人工同步|已实现|冲突、预览hash、ignore、链接、Git独立副本回归|
|Windows受限token/Low IL/ACL/Job/stdio|已实现|真实Windows内部允许、外部拒绝、取消子树和split生命周期测试|
|WASM/WIT、权限、ZIP、hook、KV|已实现并接线|真实component超限/取消/停用与安装攻击回归|
|MCP HTTP/stdio、资源、取消|已实现并接线|官方SDK本地HTTP和stdio fixtures；自动后台重连未实现|
|后台Shell、子代理spawn/fork/send/wait/interrupt|已实现并接线|协作与后台任务测试|
|Todo/Plan/Goal|事件与模型工具已实现|尚无独立管理面板|
|调度、会话压缩|应用存活期间单次调度、显式压缩服务已实现|missed不补跑、缺模型failed和原日志保留测试|
|headless/stdio JSON-RPC|已实现|真实子进程协议错误恢复和请求测试通过|
|Python/TypeScript SDK|源码已实现|Python SDK真实子进程调用通过；TypeScript SDK尚未独立发布|
|NSIS标准包|实际构建成功，12.56MiB|Windows发布版启动与单实例通过；未在干净虚拟机验证安装/卸载|
|NSIS离线包|实际构建成功，218.57MiB|已包含微软离线运行时，直连下载；未在无WebView2干净VM安装验收|
|发布签名与自动更新|Tauri插件接入，信任配置未启用|没有签名证书、更新公钥或发布端点，未发布|
|PTY/SSH执行世界/LSP/搜索provider/Office预览|未实现|不能计为P2/P3完成|
|ACP/配置预设分层/脚本工作流/签名插件市场|未实现|不能计为P2/P3完成|

数据目录为宿主独占，桌面打开时headless需使用独立数据目录，避免另一个进程执行恢复时破坏活动会话。WASM采用每次调用独立Store，activate→execute/hook→deactivate，KV成功后提交；不承诺长期驻留实例状态。
