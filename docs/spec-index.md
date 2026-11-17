# Spec 文档索引

## Spec 文档

规范设计文档统一存放于 `docs/specs/`；实施计划文档不存入 git，随任务结束直接删除。本索引只登记链接与一句话主要目标，规范的日期、修订记录与设计细节均在各规范文档内部维护。能力实现、验证与未完成状态以 [能力状态矩阵](capability-matrix.md) 为准。

| 文档 | 主要目标 |
|---|---|
| [总体规范](specs/overall.md) | 全局架构、模块职责、安全不变量、交付阶段划分与阶段验收要求。 |
| [双供应商、多协议接入](specs/multi-protocol.md) | 固定供应商目录、三协议适配、配置迁移及桌面和 CLI 接入。 |
| [P1 验收与缺口补齐](specs/p1-acceptance.md) | P1 验收范围、附件及插件缺口、安装清理和完成门槛。 |
| [CLI 命令行 Agent 调用](specs/cli.md) | `harness` 命令行 run/chat/diff/apply/models/sessions 能力，提供命令行调用入口。 |
| [ACP 协议接口](specs/acp.md) | `harness acp` 子命令：Agent Client Protocol v1 agent 端，供编辑器客户端接入。 |
| [LSP 语言服务集成](specs/lsp.md) | 只读五件套 LSP 工具（诊断/悬停/定义/引用/符号），通用 server 配置，默认 rust-analyzer。 |
| [内置浏览器集成](specs/browser.md) | Agent 会话 browser_* 工具组（headless CDP）与桌面端 WebView2 可视后端，「网页」面板人机共用同一实例。 |
| [子代理可视化与 spawn 模型选择](specs/subagent-visibility.md) | 子代理父子关系与运行状态可视化，入口为输入框下方面板；`agent_spawn` 可选 modelId。 |
| [P2 后台任务桌面可视化](specs/p2-background-tasks.md) | 后台 Shell/子代理/定时任务事件卡片渲染与会话级任务面板，宿主查询/取消/删除命令。 |
| [计划/目标管理面板](specs/plan-goal-panel.md) | 右侧只读抽屉汇总跨会话计划、目标与待办，提供 `plan_overview` 与会话跳转。 |
| [模型设置两级选择与 effort](specs/model-settings.md) | 模型设置供应商→模型两级结构、内置模型目录与启停、effort 档位与发送框级联选择。 |
| [输入框斜杠快捷指令](specs/slash-commands.md) | CLI chat 斜杠命令、行编辑与 `/` 补全弹窗；桌面端 composer 菜单与加号添加菜单。 |
| [PTY 执行世界（本地沙箱终端）](specs/pty-execution-world.md) | 沙箱内 ConPTY 持久终端与 `terminal_open/write/read/close` 工具组，自研 ConPTY + 受限 token spawn。 |
| [智谱 Coding Plan 剩余额度与模型设置布局](specs/coding-plan-usage.md) | 智谱套餐名与三张剩余额度卡，模型设置左栏分组导航/右栏详情/模型编辑弹窗化。 |
| [配置预设分层](specs/config-presets.md) | 命名会话能力配置包（模型/effort/审批策略），解析按显式参数>预设>宿主缺省三层。 |
| [模型选择方式优化](specs/model-selection.md) | CLI `/model` `/effort` 内联面板与桌面浮层卡片选择器，默认档位=最高档，切换静默持久化。 |
| [右侧面板（文件预览、改动审阅、网页浏览与计划抽屉）](specs/side-panel.md) | 桌面右侧面板承载更改审阅、文件预览、网页浏览和只读计划抽屉。 |
| [自动化任务（周期调度）](specs/automation.md) | 桌面端可管理的周期自动化任务：五类周期、可选绑定会话投递回合，独立管理面板。 |
| [Skills（技能）支持](specs/skills.md) | 技能目录与 frontmatter 解析、catalog 注入系统提示、`$name` 显式提及全文注入与禁用名单。 |
| [会话列表「最近 / 项目」视图](specs/session-list.md) | 桌面侧栏按最近/按项目两种视图，以及项目置顶、搜索、存档、草稿态、标题生成等会话管理能力。 |
| [会话用量环形指示器与上下文窗口](specs/context-usage-indicator.md) | 会话输入区用量环形与悬停明细卡，模型目录 `context_window` 元数据，ChatGPT 订阅用量接入。 |
| [每回合 Token 预算](specs/token-usage-budget.md) | 每条用户消息独立累计输出与未缓存输入 token，父子代理共享额度，默认 1M 且可调。 |
| [权限模式（五档审批策略）](specs/permission-modes.md) | 五档权限模式统一定义（ask/edit/auto/never/guarded）与 CLI/ACP/预设/桌面四端接入。 |
| [工作区（项目）命名](specs/workspace-naming.md) | 工作区显示名随 `record.json` 持久化，桌面命名弹窗与重命名入口，各显示点统一消费名称字段。 |
| [宿主代建 worktree 执行模式与沙箱提权重跑](specs/worktree-execution.md) | 工作区 worktree 执行模式：宿主代建会话 worktree、执行根切换与 shell 沙箱提权重跑。 |
| [长会话时间线性能与有界渲染](specs/long-session-timeline.md) | 桌面长会话的事件分页、正文惰性读取与虚拟时间线，作为 Tauri 与后续 GPUI 共用的时间线契约。 |
| [ask_user 提问工具](specs/ask-user.md) | 会话中模型向用户提问的 `ask_user` 内置工具：挂起等待与无人在场/超时自动结算，四端接入与时间线回放。 |
| [会话计划模式](specs/plan-mode.md) | 会话级计划模式、明确审批与拒绝反馈、右侧只读计划抽屉；shell 按正向只读白名单放行。 |
| [项目移除（隐藏入口）与重新显示](specs/project-removal.md) | 桌面移除项目=仅隐藏入口（KV 标记，数据原样保留），重开同源路径项目自动重新显示。 |
| [同源重复项目检测与合并](specs/workspace-merge.md) | 检测同一源目录的重复项目记录并支持合并：会话、自动化与定时任务改挂保留项目。 |
| [Agent 执行性能优化](specs/agent-execution-performance.md) | 工具批次并发调度、请求前自动压缩与模型 HTTP 连接复用，设置入口在桌面「性能」选项卡。 |

## 关联实现文档

| 文档 | 用途与记录范围 |
|---|---|
| [P1 验收报告](p1-acceptance-report.md) | 本次补齐、实际验证证据、产物及未通过边界。 |
| [能力状态矩阵](capability-matrix.md) | 当前已实现、已验证及未完成能力的状态。 |
| [交付记录](delivery.md) | 当前修复版安装包、验收边界及历史版本交付。 |
| [三协议交付说明](multi-protocol-delivery.md) | 双供应商配置、协议回放、使用及升级方式。 |
| [平台实现报告](platform-report.md) | 工作副本、同步、Windows 沙箱与凭据接口。 |
| [扩展实现报告](extensions-report.md) | WASM、插件权限和生命周期、MCP 接入与验证。 |
| [桌面交付报告](desktop-report.md) | 桌面界面、IPC、安装和初版验收记录；当前交付状态见能力矩阵。 |
| [安装权限修复记录](installer-uac-fix.md) | 0.1.1 安装器提权问题、修复和 PE 清单验证。 |
| [历史代码审查](review-report.md) | 审查时发现的问题，不作为当前待修问题清单。 |

## 维护约定

- 新规范设计文档只放在 `docs/specs/`，并在本索引登记相对链接与一句话主要目标；实施计划文档不存入 git，随任务结束直接删除，不登记于本索引。
- 主要目标列只登记一句话目标，不罗列功能细节、修订内容或实现状态。
- 规范修订只更新规范文档本身，日期与修订记录写在规范文档头部；本索引不随规范修订更新，仅在新增、删除、移动或改名规范时维护条目与链接。
- 实现、验证与未完成状态记录于能力状态矩阵，不把接口预留记为功能完成。
- 规范移动、改名或删除时，同步修改本索引及所有引用；明确替代关系后删除旧规范，替代关系记录于相关规范文档内部。
- 交付、进度、验证及审查记录继续放在 `docs/`，注明对应版本或记录时状态。
- 根目录 [AGENTS.md](../AGENTS.md) 保持指向本索引。
