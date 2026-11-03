# 执行记录 — docs/implementation-plan.md

## 预检
- 工作目录为空且不是已有Git仓库；已初始化feat/rust-harness，无需创建嵌套worktree。
- Task 1的contracts供平台、插件、runtime、桌面共同使用，禁止各任务私改wire字段。
- Task 2平台服务由runtime调用；Task 3插件注册的工具仍经过runtime审批。
- Ruling: 初始化项目直接放在用户指定空目录 — 没有现有分支和文件需要隔离。
- 验证证据和未完成项随实现持续记录，不将占位符计为交付。

## 2026-11-04 继续执行
- 工作副本/Windows隔离13项测试；WASM/安装/MCP18项测试；前端12项测试，在各自任务验证通过。
- 独立审阅7项Important已逐一修复，并补充红→绿回归：预览hash绑定、ignore误删除、导入事务、tool批次/未知结果、UI串工作区、插件依赖升级。
- 继续整合时发现TLS加密provider特征合并冲突，显式选择ring；全工作区测试重新运行。
- Ruling: 本机已安装工具链名stable、实际版本1.98.1，使用stable并保留MSRV，不重复下载同版本工具链。
- Ruling: npm+package-lock为唯一前端锁文件；pnpm不作为第二套锁定来源。
- Ruling: Windows integration test须嵌入Common Controls v6清单；桌面/类型构建增加栈空间，防复杂命令类型构建栈溢出。
- CLI中断前只有空main，继续后补为真实stdio JSON-RPC及SDK。ACP等尚未实现，不作完成声明。
- 状态真源：capability-matrix.md。全量日志：.cache/all-tests.log；安装构建：.cache/nsis-build.log。

## 最终整合验证
- `cargo test --workspace --offline`：51项通过，包括Common Controls清单与Specta真实导出。
- `npm test`：12项交互测试通过；`npm run build`通过；Python SDK真实子进程调用通过。
- `cargo clippy --workspace --all-targets --offline`成功，有collapsible_if/参数数量等风格警告，未宣称零警告。
- `cargo fmt --all -- --check`和`git diff --check`通过。
- Specta rc.25内联serde_json::Value导致导出无限递归，已用命名递归JsonValue及透明JsonPayload修复；扩大栈本身未解决，现测试可正常完成。
- NSIS系统缓存rename返回跨盘错误，切换官方useLocalToolsDir选项解决，不修改全局缓存配置。
- 发布版真机smoke发现未配置updater时初始化报错，改为signed-updates显式功能后，发布版启动/第二实例退出测试通过。
- 标准NSIS产物成功生成，约12.56MiB；未运行用户机器上的安装器，未声称完成干净VM安装验收。
- 离线包另用直连微软运行时构建，不使用付费代理大流量。最终状态见delivery.md。
- 离线NSIS实际生成成功：218.57MiB；标准包12.56MiB，两个版本分别保留。发布版启动与单实例smoke通过。

## 0.1.1 安装权限修复
- 用户反馈选D:\Program Files时无法写入exe。旧NSIS为currentUser，实际PE权限级别asInvoker，不会自动对失败的文件写入请求UAC。
- Ruling: 用户明确要求受保护目录安装时提权，改用官方perMachine模式，在安装开始请求管理员权限，卸载注册写HKLM；应用本身不提升权限。
- 旧标准包PE回归先失败（asInvoker），新标准包已验证requireAdministrator，主程序验证asInvoker。
- 增加PE资源验收脚本并接入Windows CI/build脚本，版本统一升为0.1.1。未实际写入用户Program Files或修改其安装注册表。
- 0.1.1标准/离线包均已实际构建，PE验证全部通过：安装器requireAdministrator，应用asInvoker。

## 0.1.2 双供应商、多协议接入

- 两个内置供应商，协议作为供应商内部选项；API Key 与代理按供应商共享，复制独立凭据引用。
- 三协议统一历史、实际请求快照、工具调用和结果、流式终态、签名/加密推理回放与跨路由隔离已实现；桌面、headless、调度接入共享配置服务。
- schema 1→2 在线备份与事务迁移，失败后保留备份并允许重试；旧配置不按地址自动归为内置。
- RED→GREEN 证据覆盖多行 SSE、截断、迁移重试、协议引用保护、复制兼容规则、未闭合 Anthropic 块、Responses 正文输入、模型错误分类及 SDK 管道关闭。
- 独立审查的三个问题已修复并复核。全工作区 75 项 Rust、23 项前端、2 项 Python SDK 真实子进程测试通过；TypeScript SDK 独立类型检查与生产构建通过；Clippy 成功但保留风格警告。
- 两个 0.1.2 NSIS 包实际生成并验证 requireAdministrator，应用 asInvoker；桌面启动和单实例验证通过。标准 12.71 MiB，离线 218.71 MiB。
- 定时器回归测试取消后台 ticker，仅手动驱动 tick，以消除测试内部竞争；未修改业务调度策略。
- 实际供应商账号联调、正式签名和干净 VM 安装/卸载未验证；完整原始路线的未实现能力保持原状态。
