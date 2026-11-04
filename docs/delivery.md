# v0.1.3 工作区输入修复

首次打开没有历史会话的工作区时，会自动创建首个会话，准备完成即可点击输入并发送。已有会话仍正常恢复；准备失败会显示错误，可点击“新建会话”重试。快速切换工作区时，在途创建会去重，迟到响应不会选中其他工作区的会话。

## 本次验证

- 新增5项交互回归，先复现失败再修复：选择新工作区、恢复空工作区、失败重试、迟到响应隔离、切走再返回时创建去重。
- 前端全部28项测试、TypeScript检查及Vite生产构建通过；独立代码审查通过。
- Windows MSVC发布版与单一NSIS安装包实际构建成功；PE清单验证安装器 requireAdministrator、应用 asInvoker。
- 目标系统使用已有WebView2，生成的安装脚本采用skip模式，无离线运行时包。
- 用户已安装的旧版应用仍在运行，本次没有中断旧会话，也未执行修复版的原生进程冒烟或交互验收。

## 安装

修复包：target/release/bundle/nsis/Rust Harness_0.1.3_x64-setup.exe，11,521,422字节（10.99 MiB）。SHA-256记录在同目录SHA256SUMS.txt。

安装前请从托盘菜单退出旧版Rust Harness，再运行安装包升级。原有数据格式保持为2，本次仅修复前端会话准备流程。

以下为历史版本交付记录。

# v0.1.2 开发预览交付记录

后续版本仅提供使用系统已有 WebView2 的单一 NSIS 安装包。下文 0.1.2 的标准/离线包是已交付历史产物，保留文件与校验值，不代表后续构建策略。

已实现 DeepSeek、智谱 Coding Plan 两个内置供应商，每个内部选择 Chat Completions、Responses、Anthropic Messages。内置端点只读，自定义复制保留兼容规则但不复制密钥。详见[三协议使用与升级说明](multi-protocol-delivery.md)。

## 本次验证

- 75 项 Rust 测试、23 项前端交互测试、2 项 Python SDK 真实子进程测试全部通过。
- 前端类型检查与生产构建、TypeScript SDK 独立类型检查、Rust 格式检查通过；Clippy 退出 0，保留风格和参数数量警告。
- 本地真实 HTTP/SSE 验证三协议工具审批、结果回传、取消、重启回放及敏感错误脱敏；未使用真实供应商 Key。
- 发布版桌面启动和单实例检查通过，使用独立临时数据目录；未修改用户真实数据。
- 标准及离线 NSIS 均已构建，实际 PE 清单验证安装器 requireAdministrator、应用 asInvoker；未在干净虚拟机执行安装/卸载。

## 安装包

| 类型 | 文件 | 大小 |
|---|---|---|
| 标准 | Rust Harness_0.1.2_x64-standard-setup.exe | 13,323,445 字节，12.71 MiB |
| 离线 | Rust Harness_0.1.2_x64-offline-setup.exe | 229,329,803 字节，218.71 MiB |

文件位于 target/release/bundle/nsis，默认 Rust Harness_0.1.2_x64-setup.exe 同标准包。该目录 SHA256SUMS.txt 包含实际文件校验值。安装包为未签名开发预览，离线包使用已有微软 WebView2 完整运行时缓存。

升级会先备份 SQLite 再迁移到格式 2；旧模型配置保留为独立自定义供应商，旧版应用不能直接打开升级后的数据库。

## v0.1 历史交付记录

0.1.1修复版曾将标准包和离线包更新为系统级提权安装。0.1.0没有请求UAC，在Program Files等受保护目录安装会失败；详情见[安装权限修复](installer-uac-fix.md)。下文的0.1.0产物保留为历史记录。

这是核心桌面应用与部分协作能力的实现交付，完整P2/P3路线仍有未完成项，详见[能力矩阵](capability-matrix.md)。

## 已验证

- 51项Rust测试、12项前端交互测试、前端生产构建。
- 实际Windows受限进程的副本写入允许、外部写入拒绝、进程树取消及stdio生命周期。
- 实际WASM组件执行、权限、内存/fuel/epoch限制和插件安装攻击检查。
- 官方MCP SDK本地HTTP/stdio握手、工具、资源、取消和重连不重放。
- 人工审批后文件仅修改副本；完整差异哈希校验后才同步原项目。
- 发布版桌面启动、第二实例退出；标准NSIS生成（非签名开发版）。

标准安装包：`target/release/bundle/nsis/Rust Harness_0.1.0_x64-setup.exe`。
离线安装包：`target/release/bundle/nsis/Rust Harness_0.1.0_x64-offline-setup.exe`，约218.57MiB，微软运行时通过直连获取。
安装包SHA-256记录位于同目录`SHA256SUMS.txt`。

## 实际边界

尚未交付真实PTY、SSH执行世界、LSP、搜索provider、Office转换、ACP、配置预设分层、Node脚本工作流、签名插件市场和签名自动更新。没有用接口占位宣称这些能力完成。

尚未使用真实付费模型API，也未在干净Windows虚拟机验收安装、卸载与无WebView2场景。Clippy已运行且成功，保留少量风格与参数数量警告。

WASM每次调用创建独立Store，长期内存状态不跨调用保留；KV成功后事务提交。MCP stdio连接持有工作副本lease，同步前需要断开。窗口关闭保留任务，应用退出仍会终止任务；停机期间定时任务不补跑。

同一数据目录只允许一个宿主。CLI和桌面同时使用时需显式指定独立数据目录。
