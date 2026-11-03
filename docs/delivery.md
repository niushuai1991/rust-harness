# v0.1 开发预览交付记录

这是核心桌面应用与部分协作能力的实现交付，完整P2/P3路线仍有未完成项，详见[能力矩阵](capability-matrix.md)。

## 已验证

- 51项Rust测试、12项前端交互测试、前端生产构建。
- 实际Windows受限进程的副本写入允许、外部写入拒绝、进程树取消及stdio生命周期。
- 实际WASM组件执行、权限、内存/fuel/epoch限制和插件安装攻击检查。
- 官方MCP SDK本地HTTP/stdio握手、工具、资源、取消和重连不重放。
- 人工审批后文件仅修改副本；完整差异哈希校验后才同步原项目。
- 发布版桌面启动、第二实例退出；标准NSIS生成（非签名开发版）。

标准安装包：`target/release/bundle/nsis/Rust Harness_0.1.0_x64-setup.exe`。
离线构建记录：`.cache/nsis-offline.log`；如果成功，独立产物命名为`Rust Harness_0.1.0_x64-offline-setup.exe`。

## 实际边界

尚未交付真实PTY、SSH执行世界、LSP、搜索provider、Office转换、ACP、配置预设分层、Node脚本工作流、签名插件市场和签名自动更新。没有用接口占位宣称这些能力完成。

尚未使用真实付费模型API，也未在干净Windows虚拟机验收安装、卸载与无WebView2场景。Clippy已运行且成功，保留少量风格与参数数量警告。

WASM每次调用创建独立Store，长期内存状态不跨调用保留；KV成功后事务提交。MCP stdio连接持有工作副本lease，同步前需要断开。窗口关闭保留任务，应用退出仍会终止任务；停机期间定时任务不补跑。

同一数据目录只允许一个宿主。CLI和桌面同时使用时需显式指定独立数据目录。
