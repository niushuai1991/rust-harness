# 能力状态（2026-11-08）

状态说明：已实现表示存在实际执行代码；已验证另需对应测试或实际构建证据。不以计划或接口占位替代功能。

|能力|实际状态|验证|
|---|---|---|
|Rust/Tauri桌面、中文聊天与设置|已实现并验证|0.1.4前端34项测试与生产构建通过；Windows 11 guest 中完成输入、审批、同步、托盘、单实例与重启继续会话|
|DeepSeek/智谱双供应商与Chat/Responses/Anthropic|已实现并接线|六条固定路由、三协议本地HTTP/SSE工具审批及重启回放、复制兼容与凭据隔离；两供应商六条真实连接及 Agent 工具链路已通过；模型设置两级结构、智谱 Coding Plan 额度卡、模型选择交互的桌面 UI 与真实模型/额度抽查待用户，见[模型设置规范](specs/model-settings.md)、[Coding Plan 额度规范](specs/coding-plan-usage.md)、[模型选择规范](specs/model-selection.md)|
|ChatGPT 账号供应商（浏览器 OAuth 登录、token 自动刷新与吊销、ChatGPT 后端 Responses 变体、CLI login/logout 与桌面登录卡片）|已实现并接线|OAuth 模块 16 项单测与 6 项 mock issuer e2e（回调保存/state 校验/取消/刷新轮换/永久失败）通过；真实抽查通过：Chrome 授权登录、`login status`、gpt-5.6-luna 真实流式回合、effort 档位、工具闭环（write→审批→read）、桌面已登录状态两端共享。已知缺口：内置模型目录为 codex 全量列表，free 套餐仅部分模型可用（其余 400），待按账号从 /models 过滤；国内访问需代理（CLI 走 HTTPS_PROXY 环境变量，桌面走供应商代理字段）|
|SQLite WAL、游标、恢复、FTS、JSONL、附件/spill|已实现并验证|持久顺序、未知结果结算、导入事务；schema2备份迁移、回滚和重试；附件导入、会话隔离、分块读取、下载与重启回归|
|原项目/执行副本分离与人工同步|已实现|冲突、预览hash、ignore、链接、Git独立副本回归；桌面右侧面板行级diff审阅与文件预览已实现（见[右侧面板规范](specs/side-panel.md)），UI抽查待用户|
|Windows受限token/Low IL/ACL/Job/stdio|已实现|真实Windows内部允许、外部拒绝、取消子树和split生命周期测试|
|WASM/WIT、权限、ZIP、hook、KV、声明式界面|已实现并接线|真实component超限/取消/停用与安装攻击；设置、审批命令、受控HTTP、主题和安全卡片回归|
|MCP HTTP/stdio、资源、取消|已实现并接线|官方SDK本地HTTP和stdio fixtures；自动后台重连未实现|
|后台Shell、子代理spawn/fork/send/wait/interrupt|已实现并接线|协作与后台任务测试；桌面端事件卡片与会话级任务面板（查询/取消/删除定时）见[P2后台任务规范](specs/p2-background-tasks.md)，桌面 UI 抽查待用户|
|子代理可视化与spawn模型选择|已实现并接线|父子关系重建与modelId校验3项测试、前端3项测试通过；会话列表子任务徽标、事件卡片跳转、运行状态live-dot；ACP透传与管理面板未实现|
|会话列表「最近 / 项目」视图与项目置顶|已实现并接线|storage 置顶 KV 3 项、sessionList 纯函数 8 项、App 视图/置顶/搜索 3 项新增测试通过；fmt/clippy/build 与 npm test（161 项）全量通过，cargo test 仅 `chatgpt_models_require_login_before_build` 失败（main 既有，与本能力无关）；2026-11-09 侧栏会话入口重整（移除工作区下拉框与标题行 +，新增「新建聊天」按钮与项目组头新建，App 测试 51 项全通过）；见[会话列表视图规范](specs/session-list.md)；桌面 UI 抽查待用户|
|Todo/Plan/Goal|事件与模型工具已实现，会话流渲染清单/计划/目标卡片；桌面「计划与目标」面板已实现并接线（跨会话只读汇总、演变历史、跳转会话，见[面板规范](specs/plan-goal-panel.md)）|storage 派生查询 2 项与前端 6 项新增测试通过；面板为只读派生视图，UI 手动写入不在范围，桌面 UI 抽查待用户|
|调度、会话压缩|应用存活期间单次调度、显式压缩服务已实现；桌面端周期自动化任务（五类周期、绑定会话投递、管理面板）已实现并接线，见[自动化任务规范](specs/automation.md)|missed不补跑、缺模型failed和原日志保留测试；自动化任务桌面 UI 抽查待用户|
|headless/stdio JSON-RPC|已实现|真实子进程协议错误恢复和请求测试通过|
|CLI 命令行 run/chat/diff/apply/models/sessions|已实现并接线|真实模型端到端验证通过（run 流式/JSONL 契约/diff-apply 闭环/host.lock 互斥/子代理作用域）；chat 交互与 ask 审批 y/n 抽查待用户（依赖 TTY），见[CLI 命令行规范](specs/cli.md)|
|ACP v1 agent 端（harness acp）|已实现并接线|官方SDK 5.0.0；16项映射/fixture单测与11项duplex集成测试（initialize门禁、审批ask双路径、取消、load回放、子进程冒烟）通过；真实模型端到端流式chunk、end_turn与重启load回放通过；mcpServers参数忽略，Zed实测待用户|
|Python/TypeScript SDK|源码已实现|Python SDK真实子进程2项通过；TypeScript SDK独立类型检查通过，尚未独立发布|
|NSIS安装包|0.1.4单包11.17MiB，使用系统已有WebView2|Windows 11 guest 中完成 0.1.2 安装、0.1.4 覆盖升级、默认卸载保留数据及显式清理三固定根；外部目标保留|
|NSIS离线包|仅保留0.1.2及以前的历史产物，后续不再提供|后续仅构建系统已有WebView2的单一NSIS包（skip模式）|
|发布签名与自动更新|Tauri插件接入，信任配置未启用|没有签名证书、更新公钥或发布端点，未发布|
|LSP只读五件套（诊断/悬停/定义/引用/符号）|已实现并接线|mock server 11项与runtime流程6项测试；真实rust-analyzer经Windows沙箱冒烟通过（E0308诊断与hover到位），测试为`#[ignore]`仅显式运行，见[LSP规范](specs/lsp.md)|
|内置浏览器（browser_navigate/snapshot/click/type/press/screenshot/evaluate/close）|已实现并接线|11项Rust测试（白名单/stub解析/命令契约/审批链路，含真实Edge headless全链路）通过；真实模型端到端（glm-5.3-flash）navigate→snapshot→type→click→evaluate读到预期文本，turn completed，见[浏览器规范](specs/browser.md)|
|PTY 本地沙箱终端（terminal_open/write/read/close）|已实现并接线|ANSI 过滤/有界缓冲 8 项单测、orchestration stub 全流程 3 项与前端 52 项测试通过；真实 ConPTY 集成测试（往返/杀树/租约）在本开发机预检跳过——该机 ConPTY 客户端连接不可用（0xC0000142，最小样例复现，见[PTY规范](specs/pty-execution-world.md)），在 GitHub CI windows runner 真实运行；真实模型端到端待用户抽查；SSH 远端执行世界未实现，不能计为 P3 完成|
|配置预设（模型/effort/审批策略命名组合）|已实现并接线|契约 3 项、PresetService 6 项、审批分层 6 项单测与 CLI 链路/ACP 2 项集成测试先红后绿；真实 `harness run --preset` 回合完成；桌面 3 项面板测试与发送框级联填充测试通过；解析为显式参数>预设>宿主缺省三层，桌面审批保持交互式，见[配置预设规范](specs/config-presets.md)，桌面 UI 抽查待用户|
|Skills 技能（.agents/skills 目录、catalog 注入、$name 提及、启停）|已实现并接线|skills 模块 17 项单测与 agent_flow 2 项集成（catalog 进 system、$ 注入与 $PATH 排除、禁用生效）先红后绿；CLI skills 子命令//skills 与 $ 补全 6 项测试、前端 13 项（skills 纯函数+SkillsPanel/EventCard）通过；真实模型端到端与桌面 UI 抽查待用户，见[Skills 支持规范](specs/skills.md)|
|输入框斜杠快捷指令（/ 命令注册表、补全弹窗、加号添加菜单）|已实现并接线|CLI 弹窗/过滤/补全/命令与真实模型回合、桌面菜单交互真实抽查通过（2026-11-06）；2026-11-07 扩展的 /plan /goal 命令与加号添加菜单 UI 抽查待用户，见[斜杠快捷指令规范](specs/slash-commands.md)|
|权限模式（五档审批策略 ask/edit/auto/never/guarded）|已实现并接线|contracts 决策矩阵与危险命令规则 19 项单测、runtime 会话策略/默认档/子代理继承 6 项集成、CLI approval 6 项与 ACP modes 测试、前端模式选择器 2 项测试通过；四端接入（CLI --approval、预设字段、ACP modes、桌面选择器+KV 默认档）接线完成；真实会话各档行为与桌面 UI 抽查待用户，见[权限模式规范](specs/permission-modes.md)|
|搜索provider/Office预览|未实现|不能计为P2/P3完成|
|脚本工作流/签名插件市场|未实现|不能计为P3完成|

数据目录为宿主独占，桌面打开时headless需使用独立数据目录，避免另一个进程执行恢复时破坏活动会话。WASM采用每次调用独立Store，activate→execute/hook→deactivate，KV成功后提交；不承诺长期驻留实例状态。

0.1.2 完整验证：75 项 Rust、23 项前端、2 项 Python SDK；详见[三协议交付说明](multi-protocol-delivery.md)。

0.1.3修复首次选择空工作区时输入框禁用，新增5项回归并通过前端全部28项测试；单安装包及边界见[交付记录](delivery.md)。
