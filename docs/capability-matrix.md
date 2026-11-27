# 能力状态（2026-11-18）

状态说明：已实现表示存在实际执行代码；已验证另需对应测试或实际构建证据。不以计划或接口占位替代功能。

|能力|实际状态|验证|
|---|---|---|
|Agent Hooks（命令处理器）|阶段性实现|用户数据目录全局配置已接入部分事件；Windows 进程与运行时集成测试覆盖部分路径。项目配置仅阻断，审核和撤销未实现；插件 Hook 授权与执行、会话快照和完整事件契约尚未完成，见[设计草稿](specs/agent-hooks-draft.md)|
|运行中插话|已实现（桌面端待真实模型验收）|模拟模型覆盖同回合输入、去重和取消；模拟持久化受理后内存登记未完成的场景；存储覆盖恢复和跨页状态；前端覆盖发送、重试、后台事件、工具配对和分页。真实桌面与模型端到端待验收，见[运行中插话规范](specs/turn-steering.md)|
|无项目会话与无项目自动化|已实现（待验证）|storage 哨兵创建/过滤、platform scratch root 幂等与非法 id 拒绝、runtime 无项目回合独立执行根与上下文标注、orchestration 无项目自动化创建/列表/手动执行、前端 chip「无项目」落点/侧栏分组/面包屑/组头入口收敛等用例已编写；开发会话为嵌套沙箱，依赖 Windows 沙箱 ACL 授权（SetNamedSecurityInfoW）的集成用例本环境不可执行（未改动代码同样失败，已复现确认），待 CI/正常环境全量验证，见[无项目会话规范](specs/no-project-sessions.md)|
|Rust/Tauri桌面、中文聊天与设置|已实现并验证|0.1.4前端34项测试与生产构建通过；Windows 11 guest 中完成输入、审批、同步、托盘、单实例与重启继续会话|
|DeepSeek/智谱双供应商与Chat/Responses/Anthropic|已实现并接线|六条固定路由、三协议本地HTTP/SSE工具审批及重启回放、复制兼容与凭据隔离；两供应商六条真实连接及 Agent 工具链路已通过；模型设置两级结构、智谱 Coding Plan 额度卡、模型选择交互的桌面 UI 与真实模型/额度抽查待用户，见[模型设置规范](specs/model-settings.md)、[Coding Plan 额度规范](specs/coding-plan-usage.md)、[模型选择规范](specs/model-selection.md)|
|ChatGPT 账号供应商（浏览器 OAuth 登录、token 自动刷新与吊销、ChatGPT 后端 Responses 变体、CLI login/logout 与桌面登录卡片）|已实现并接线|OAuth 模块 16 项单测与 6 项 mock issuer e2e（回调保存/state 校验/取消/刷新轮换/永久失败）通过；真实抽查通过：Chrome 授权登录、`login status`、gpt-5.6-luna 真实流式回合、effort 档位、工具闭环（write→审批→read）、桌面已登录状态两端共享。已知缺口：内置模型目录为 codex 全量列表，free 套餐仅部分模型可用（其余 400），待按账号从 /models 过滤；国内访问需代理（CLI 走 HTTPS_PROXY 环境变量，桌面走供应商代理字段）|
|SQLite WAL、游标、恢复、FTS、JSONL、附件/spill|已实现并验证|持久顺序、未知结果结算、导入事务；schema2备份迁移、回滚和重试；附件导入、会话隔离、分块读取、下载与重启回归|
|回合回复呈现|已实现并验证|完成回合过程默认折叠，轮尾复制按顺序合并助手正文；用户消息、异常回合和回合外事件不显示复制按钮；跨页回合按需读取整轮正文，前端回归覆盖状态切换、跨页及异常处理；桌面 UI 抽查待用户，见[回合回复呈现规范](specs/turn-reply-presentation.md)|
|长会话有界时间线（第一期）|已实现并验证|桌面改用 `session_timeline_page`：默认 80 条、硬上限 200 条、严格向前游标；活动会话历史不再限制页数，滚动到顶部自动加载、按钮可手动加载，可一直向前读到会话开头（显示「已到会话开头」），实时事件满页时另起一页而不丢事件；非活动会话按最多 4 个会话的 LRU 释放，主会话与抽屉中打开的侧聊固定不淘汰。storage 游标边界、Tauri cursor 解析、Zustand 不限页数分页与满页不丢事件、App 滚动加载/切换竞态均有回归覆盖，前端全量 315 项测试与生产构建通过；可变高度虚拟列表、摘要化和断线补拉仍未实现，见[长会话时间线规范](specs/long-session-timeline.md)|
|原项目/执行副本分离与人工同步|已实现|冲突、预览hash、ignore、链接、Git独立副本回归；桌面右侧面板行级diff审阅与文件预览已实现（见[右侧面板规范](specs/side-panel.md)），UI抽查待用户|
|Windows受限token/Low IL/ACL/Job/stdio|已实现|真实Windows内部允许、外部拒绝、取消子树和split生命周期测试|
|WASM/WIT、权限、ZIP、hook、KV、声明式界面|已实现并接线|真实component超限/取消/停用与安装攻击；设置、审批命令、受控HTTP、主题和安全卡片回归|
|MCP 客户端配置、stdio/HTTP、会话隔离与取消|已实现，桌面真实模型待验收|配置持久化与信任、同名隔离、取消/不重放、配置竞争、计划模式及 direct/replica/worktree/scratch 的受控测试；文件系统服务器 2025.8.21 本地互操作与 SDK 1.32.2 的 HTTP 受控服务通过。OAuth、资源模板、会话资源工具和 ACP 传入配置尚未实现；图片与音频暂以说明替代，见[MCP 规范](specs/mcp.md)|
|后台Shell、子代理spawn/fork/send/wait/interrupt|已实现并接线|协作与后台任务测试；`background_run` 按会话执行位置执行（直接/worktree/副本/scratch，直接会话下不再报 INVALID_WORKSPACE），shell 前台到限（`app.tool_timeout_secs` 可配置）自动转受管后台任务继续运行；桌面端事件卡片、会话级任务面板与输入框终端图标浮层（运行中任务+时长）见[P2后台任务规范](specs/p2-background-tasks.md)，桌面 UI 抽查待用户|
|子代理可视化与spawn模型选择|已实现并接线|父子关系重建与modelId校验3项测试、前端3项测试通过；会话列表子任务徽标、事件卡片跳转、运行状态live-dot；ACP透传与管理面板未实现|
|会话列表「最近 / 项目」视图与项目置顶|已实现并接线|storage 置顶 KV 3 项、sessionList 纯函数 8 项、App 视图/置顶/搜索 3 项新增测试通过；fmt/clippy/build 与 npm test（161 项）全量通过，cargo test 仅 `chatgpt_models_require_login_before_build` 失败（main 既有，与本能力无关）；2026-11-09 侧栏会话入口重整（移除工作区下拉框与标题行 +，新增「新建聊天」按钮与项目组头新建，App 测试 51 项全通过）；2026-11-13 启动默认进入新会话草稿态（不自动打开最近会话），前端全量 274 项测试与生产构建通过；见[会话列表视图规范](specs/session-list.md)；桌面 UI 抽查待用户|
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
|内置浏览器（browser_navigate/snapshot/click/type/press/screenshot/evaluate/close）|已实现并接线|11项Rust测试（白名单/stub解析/命令契约/审批链路，含真实Edge headless全链路）通过；真实模型端到端（glm-5.3-flash）navigate→snapshot→type→click→evaluate读到预期文本，turn completed，见[浏览器规范](specs/browser.md)。2026-11-16 契约新增 forward（action=forward，真实浏览器后退→前进用例通过）并落地桌面 WebView2 可视后端：「网页」面板与 browser_* 工具人机共用同一实例、后退/前进/刷新工具栏、模型操作自动弹出面板；后端单测与前端 vitest 全过，真实桌面 UI 抽查待用户|
|PTY 本地交互终端（terminal_open/write/read/close）|已实现并接线|ANSI 过滤/有界缓冲单测、orchestration stub 全流程与前端测试通过；2026-11-22 终端扩展到全部执行位置（worktree/scratch 沙箱内、直接模式宿主令牌+每条输入审批，弃用 sandbox_lease 路径反推并删除 ShellService::run 死代码），真实 ConPTY 集成测试（往返/杀树/租约/worktree/scratch/direct/退出码探测）本开发机 6 项通过（此前 0xC0000142 预检跳过，本次预检通过），CI windows runner 亦真实运行；2026-11-22 真实桌面抽查暴露并修复 terminal_write 阻塞（try_exit_code 非阻塞探测），GUI 桌面宿主 ConPTY 附着在 Insider 26300 上全面失效（矩阵探针消元：两令牌路径皆坏，24H2 conhost 回归；CREATE_NO_WINDOW 已防弹窗）；2026-11-22 根治落地：优先侧载官方 ConPTY NuGet 的 conpty.dll/OpenConsole.exe 配对（缺失回退 inbox），开发机 OpenConsole 路径真实 PTY 集成 6/6 附着成功，详见 PTY 规范边界节；真实模型端到端待用户抽查；SSH 远端执行世界未实现，不能计为 P3 完成，见[PTY规范](specs/pty-execution-world.md)|
|配置预设（模型/effort/审批策略命名组合）|已实现并接线|契约 3 项、PresetService 6 项、审批分层 6 项单测与 CLI 链路/ACP 2 项集成测试先红后绿；真实 `harness run --preset` 回合完成；桌面 3 项面板测试通过；解析为显式参数>预设>宿主缺省三层，桌面审批保持交互式；2026-11-21 移除桌面发送框预设下拉（预设经设置页面板管理与三端引用，发送框级联填充测试随之移除），见[配置预设规范](specs/config-presets.md)，桌面 UI 抽查待用户|
|Skills 技能（.agents/skills 目录、catalog 注入、$name 提及、启停）|已实现并接线|skills 模块 17 项单测与 agent_flow 2 项集成（catalog 进 system、$ 注入与 $PATH 排除、禁用生效）先红后绿；CLI skills 子命令//skills 与 $ 补全 6 项测试、前端 13 项（skills 纯函数+SkillsPanel/EventCard）通过；真实模型端到端与桌面 UI 抽查待用户，见[Skills 支持规范](specs/skills.md)|
|全局指令文件（数据目录 AGENTS.md 注入系统提示）|已实现并接线|agent_flow 3 项集成（全局注入且先于项目级、32000 字符截断、空白内容静默跳过）先红后绿；runtime 层注入，CLI/桌面/ACP/子代理自动生效；真实模型端到端抽查待用户，见[全局指令文件规范](specs/global-agents-md.md)|
|输入框斜杠快捷指令（/ 命令注册表、补全弹窗、加号添加菜单）|已实现并接线|CLI 弹窗/过滤/补全/命令与真实模型回合、桌面菜单交互真实抽查通过（2026-11-06）；2026-11-07 扩展的 /plan /goal 命令与加号添加菜单 UI 抽查待用户，见[斜杠快捷指令规范](specs/slash-commands.md)|
|每回合 Token 预算（父子代理共享、UsageRing 可调）|已实现并接线|三协议用量归一化、上限工具跳过、缺失 usage 失败关闭、子代理共享预算与设置持久化集成覆盖；Rust 全工作区 all-features 测试、clippy、build、fmt 检查通过；前端全量 223 项测试与生产构建通过；自动标题一次性请求不计入回合预算，桌面 UI 抽查待用户，见[每回合 Token 预算规范](specs/token-usage-budget.md)|
|权限模式（五档审批策略 ask/edit/auto/never/guarded）|已实现并接线|contracts 决策矩阵与危险命令规则单测（Guarded 闸门 2026-11-22 扩展到命令承载类工具：shell/background_run 同守危险规则、terminal_open 一律询问）、runtime 会话策略/默认档/子代理继承与 Guarded 后台命令点名规则集成、CLI approval 与 ACP modes 测试、前端模式选择器测试通过；审批理由按直接模式区分文案；四端接入接线完成；真实会话各档行为与桌面 UI 抽查待用户，见[权限模式规范](specs/permission-modes.md)|
|ask_user 提问工具（Agent 向用户提问）|已实现并接线|contracts serde 3 项、runtime 挂起/无人在场/子代理路由 3 项集成、CLI 输入解析 3 项与 console 渲染 4 项、前端提问 Modal/回放配对/隐藏思考 8 项测试通过；CLI chat 交互渲染、run 非 TTY unattended 结算、JSON-RPC ask.list/resolve、桌面 question_list/resolve 与时间线回放卡接线完成；ACP v1 以 unattended 自动结算（elicitation 转发未实现）；真实模型端到端与桌面 UI 抽查待用户，见[ask_user 提问工具规范](specs/ask-user.md)|
|项目移除（隐藏入口）与重新显示|已实现并接线|storage 隐藏 KV 往返/去重/跨重启与隐藏不影响会话恢复 2 项测试先红后绿；`workspace_remove` 仅置隐藏标记（运行中确认停止）、`workspace_reveal_hidden` 重开同路径重新显示、`WorkspaceInfo.hidden` KV 派生注入；前端右键菜单/隐藏确认/停止运行参数/重新显示提示/存档管理隐藏项目可查看可恢复 4 项测试，npm test 272 项与生产构建通过；cargo fmt/clippy/build 通过，cargo test 仅既有环境性失败（真实浏览器导航、git 子进程 ×2，主工作区复验同失败）；真实桌面 UI 抽查待用户，见[项目移除规范](specs/project-removal.md)|
|搜索provider/Office预览|未实现|不能计为P2/P3完成|
|脚本工作流/签名插件市场|未实现|不能计为P3完成|
|Agent 执行性能优化（工具批次并发调度、请求前自动压缩、HTTP 连接复用）|已实现并接线|并发段重叠/上限/屏障/取消 unknown 结算/read-before-edit（6 项）、自动压缩阈值触发/投影重置/失败语义/预算记账（4 项）、连接 keep-alive 复用与传输配置隔离（3 项）集成测试通过；桌面设置「性能」选项卡 5 项前端测试通过；真实模型收益基准对照待用户；流式提前执行未实施（待预算规范修订），见[Agent 执行性能优化规范](specs/agent-execution-performance.md)|
|直接执行模式与会话级执行位置|已实现并接线|新建项目默认直接模式（不复制、不 prepare、不建 worktree），worktree 改为会话级 opt-in；会话固化执行位置、子代理复用父 worktree；直接模式 shell 走宿主令牌与五档审批。平台/存储/运行时/前端用例见[直接执行规范](specs/direct-execution.md)；桌面真实会话抽查待用户|
|会话计划模式（只读探索、只读 shell 放行、子代理调研放行）|已实现并接线|持久化/重启恢复/worktree 拒绝/未验证历史/无人值守 plan_exit 等会话计划 15 项测试、只读 shell 免审批放行/写命令与重定向拒绝/提权拒绝/沙箱不提权重跑测试、子代理调研放行 6 项测试（spawn 免审批放行与子会话强制只读、受信 agent_wait 按名暴露且未受信扩展不可执行、真实 agent_wait 报告回流、运行中调研子代理拒绝重进、结束后放行重进、非计划模式审批不变）通过；fmt/clippy/test/build 全量通过；真实模型与桌面 UI 抽查待用户，见[会话计划模式规范](specs/plan-mode.md)|

数据目录为宿主独占，桌面打开时headless需使用独立数据目录，避免另一个进程执行恢复时破坏活动会话。WASM采用每次调用独立Store，activate→execute/hook→deactivate，KV成功后提交；不承诺长期驻留实例状态。

0.1.2 完整验证：75 项 Rust、23 项前端、2 项 Python SDK；详见[三协议交付说明](multi-protocol-delivery.md)。

0.1.3修复首次选择空工作区时输入框禁用，新增5项回归并通过前端全部28项测试；单安装包及边界见[交付记录](delivery.md)。
