# 无项目会话与无项目自动化

- 日期：2026-11-18（计划批准）；修订：2026-11-26（无项目会话支持全局 MCP）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面端允许在不选择任何项目的情况下创建会话与自动化任务（「无项目」）；数据层以 `workspace_id` 空串为哨兵值；无项目会话的执行根为数据目录下按会话独立分配的 scratch 目录；侧栏分组、草稿态交互与相关面板的适用性调整。

## 背景与动机

此前桌面端创建会话必须先选择（或新建）一个项目：`session_create` 强制校验工作区存在，草稿态项目 chip 菜单只列出项目候选，自动化任务同样必须绑定项目。这把「临时问个问题」「不想为一次性任务建项目」的场景挡在了门外。

本规范引入「无项目」作为一等落点：项目不再是创建会话的前置条件，但既有的项目隔离副本、worktree 执行与审阅同步语义对已关联项目的会话保持不变。

## 设计

### 哨兵值与数据层

- `sessions.workspace_id` 为空串（`''`）表示无项目会话（哨兵值）。该列为无外键的纯文本标识，因此不引入 schema 迁移（v4 不变）。
- `Store::create_session` 放宽空工作区校验；`sessions_filtered(Some(""))` 按哨兵精确过滤（只返回无项目会话），`sessions(None)` 全量语义不变；`move_sessions` 的工作区参数仍要求非空（项目合并不作用于哨兵）。
- 无项目会话支持标题状态、重命名、归档/恢复等既有会话语义，与普通会话无差别。

### 执行根（scratch root）

- 无项目会话的执行根为 `<数据目录>/sessions/<会话 id>/root`，由 `WorkspaceManager::ensure_scratch_root(session_id)` 懒创建并打沙箱授权（capability 派生自会话 id），幂等；会话 id 必须为 UUID，防止路径注入。
- scratch root 不注册为工作区记录：不参与差异审阅、同步与副本语义；模型上下文（`context.snapshot`）明确标注无项目语义。
- 回合执行对哨兵会话跳过工作区租约（`run_session` 的 acquire 分支）；计划模式对哨兵会话无 worktree 预建语义，其余计划模式逻辑照常。
- 后台任务与终端两个模型工具（`background_run` / `terminal_open`）对哨兵会话同样使用 scratch root 且不持租约（`TerminalEntry._lease` 与后台任务租约改为 `Option`）。

### 桌面命令

- `session_create { workspaceId?: string, title }`：`workspaceId` 缺省或空串表示无项目；非空时保留工作区存在性校验。
- `skill_list` / `skill_set_enabled` 对哨兵工作区按用户主目录处理（仅用户级技能，沿用全局禁用名单）。
- `workspace_diff`、`lsp_restart` 对哨兵返回明确的「暂不支持」错误；插件贡献/命令/文件预览与工作区文件预览同理。

### 自动化任务

- 自动化的 `workspace_id` 允许空串哨兵：创建、列表过滤（按空串过滤得到无项目自动化）、编辑与手动执行照常。
- 到期执行未绑定会话时按哨兵 `create_session("")` 新建无项目会话，随后复用运行时的哨兵执行分支；绑定的会话必须与自动化同工作区（字符串比较天然覆盖哨兵）。

### 前端交互

- 新建聊天草稿态的项目 chip 菜单顶部提供固定「无项目」项；选中后 chip 显示「无项目」，欢迎语与输入框占位文案随之变化（如「发送后将在 无项目 创建会话」）。
- 发送首条消息时以空串落点调用 `session_create`；创建后的会话归入侧栏「无项目」虚拟分组。
- 侧栏「按项目」视图新增「无项目」组：虚拟工作区（id 为空串、名称「无项目」），固定排在全部项目组之后；不提供置顶、重命名与右键项目操作，组头保留折叠、计数与「新建聊天」。
- 「按最近」视图的会话行第二行项目名显示「无项目」；面包屑在无项目会话激活时显示「无项目」。
- 无项目会话激活时：右侧面板「更改 / 预览」选项卡禁用并提示原因；自动化徽标与自动化面板照常按哨兵工作区拉取与创建；技能补全按用户级技能工作。
- 启动时若上次项目不可用且项目列表为空，进入无项目草稿态（不强制先建项目）。

## 非目标

- 不提供无项目会话迁移到项目的能力（后续另行立项）。
- 无项目会话不提供项目级 MCP 配置、语言服务器、插件命令与差异审阅；全局 MCP 配置按 [MCP 客户端支持](mcp.md) 生效，能力状态以矩阵为准。
- CLI / JSON-RPC / ACP 不暴露哨兵语义（三宿主的会话创建入口天然带工作区）。
- 会话导入仍要求显式工作区，不支持导入为无项目会话。
- 无项目会话的执行根不做差异记录与回滚，删除会话不清理 scratch 目录（后续可另行立项清理策略）。

## 验证

- `harness-storage`：哨兵创建与过滤、既有会话语义、`move_sessions` 非空校验（`tests/no_project.rs`）。
- `harness-platform`：scratch root 懒创建、幂等、每会话独立、非法 id 拒绝、不注册工作区记录（`tests/workspaces.rs::scratch_roots_are_lazy_unique_and_idempotent`）。
- `harness-runtime`：哨兵会话回合在独立 scratch root 内执行工具、系统上下文标注无项目语义（`tests/no_project.rs::scratch_session_runs_in_per_session_root`）。
- `harness-orchestration`：无项目自动化创建/列表/手动执行、绑定无项目会话合法（`src/tests.rs::automation_supports_no_project_workspace_sentinel`）。
- 前端：chip 菜单「无项目」选择与发送、侧栏无项目组、面包屑、组头入口收敛（`App.test.tsx` 无项目会话分组）与分组纯函数（`sessionList.test.ts`）。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。

## 环境限制说明（开发会话记录）

部分集成测试（依赖 Windows 沙箱 ACL 授权 `SetNamedSecurityInfoW` 的用例）在嵌套沙箱的开发会话内因权限不足无法执行；这些用例在正常权限环境（CI 与桌面运行时）通过。相关失败与本次改动无关，已用未改动代码复现确认。
