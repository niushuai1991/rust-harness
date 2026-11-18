# 直接执行模式与会话级执行位置

- 日期：2026-11-18（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：新增「直接模式」（执行根=源项目目录，无副本、无 worktree、shell 无沙箱），作为新建项目的默认；worktree 从工作区级自动模式改为会话级可选；会话固化的执行位置与旧数据兼容规则。相关：[worktree 执行模式](worktree-execution.md)、[权限模式](permission-modes.md)。

## 背景与决策

此前执行位置由工作区记录 `execution_mode` 单值决定：

- `replica`：创建项目时复制整个源目录，所有会话共享副本，改动经差异审阅后 `workspace_apply` 同步回源项目。
- `worktree`：工作区一旦设为 worktree，**每个会话与每个子代理**首个 turn 启动前都会在 `<源项目>/.worktrees/` 代建一个独立 worktree。

实际使用中两个成本都不合理：创建项目时复制整棵源码树耗时且占空间；worktree 工作区里即使只是问答、读代码，也会为每个会话（含子代理）创建并保留 worktree 目录与分支，且没有自动清理入口。用户确认两项调整：

1. 新增「直接模式」作为默认执行位置：新会话直接以源项目目录为执行根，不复制、不 prepare、不建任何目录。
2. worktree 改为**会话级 opt-in**：只有新建会话时显式选择才创建，不再由工作区设置自动套用到所有会话。
3. 直接模式的 shell **不使用 Windows 受限令牌沙箱**（不对源项目目录打 DACL），以宿主令牌执行，写操作由五档审批策略约束。

## 执行位置解析

`ExecutionMode` 扩展为三值：`direct` | `replica` | `worktree`。运行时按以下优先级解析会话执行位置：

1. 无项目会话（`workspace_id` 为空串哨兵）：沿用会话独立 scratch root（[无项目会话规范](no-project-sessions.md)）。
2. 会话记录固化的 `execution_mode`（新建会话时写入）。
3. 旧会话（会话字段为空）且命中工作区 `record.json` 的 `worktrees` 登记：按登记的 worktree 执行，存量会话不漂移。
4. 工作区默认 `execution_mode`。

三种执行位置的行为：

| 模式 | 执行根 | 准备动作 | shell |
|---|---|---|---|
| `direct` | 源项目目录 | 无：不复制、不 prepare、不建目录 | 宿主令牌直接执行，审批沿用五档 |
| `replica` | `<data>/workspaces/<id>/root` | 创建工作区时复制（现状） | 受限令牌沙箱（现状） |
| `worktree` | `<source>/.worktrees/harness-<会话 id 前 8 位>` | 会话首个 turn 前懒建（现状，仅会话选择 worktree 时） | 受限令牌沙箱（现状） |

## 新建默认与存量兼容

- 新建工作区默认 `direct`：桌面创建弹窗、CLI `--exec-mode` 缺省值与 JSON-RPC `workspace.open` 缺省值均改为 `direct`。旧 `record.json` 中的 `replica` / `worktree` 值保持不变，不做迁移写盘。
- 项目「编辑」弹窗收敛为 `direct` / `replica` 两项；旧 `worktree` 工作区在弹窗中显示为 `direct`，保存后按新值落盘。已登记的会话 worktree 目录、分支与 `worktrees` 登记全部保留，不自动删除。
- 新建会话时固化执行位置：桌面草稿态提供「执行位置」选择，默认值取工作区模式，但工作区为旧 `worktree` 值时默认 `direct`。未显式指定的调用方（CLI、自动化、ACP、fork 的旧路径）写入空值，按上面的优先级回退。
- 子代理与会话 fork：继承父会话固化的执行位置。子代理不写自己的 worktree 登记，运行时按父会话登记解析同一 worktree 与 capability（同路径、同分支、同 capability），不再各自新建；fork 出的会话在首个 turn 时懒建自己的 worktree（独立副本，不在 fork 命令内写记录，避免与回合租约冲突）。
- 计划模式守卫 `PLAN_MODE_WORKTREE` 只对有效执行位置为 worktree 且尚未登记的会话生效。

## 直接模式

- 执行根为工作区 `source_path` 原样目录；不创建 `<data>/workspaces/<id>/root` 副本，`replica_path` 仅作记录占位满足加载校验。`record.json` 的 baseline 为空快照。
- 文件工具（read/write/edit/list_files/search）直接作用于源项目目录，read-before-edit 与唯一匹配校验逻辑不变。写工具的结果文案显示「已写入/已修改 <path>」，不再出现「副本」字样。
- shell 以宿主令牌执行（`ShellService::run_in` 的 `sandboxed=false` 路径），工作目录为源项目目录；不使用 `sandbox_lease`、不调用 `shell::prepare`，因此不会在源项目目录树写入 capability ACE、不递归遍历目录。命令审批完全由[权限模式](permission-modes.md)的 `ApprovalPolicy::decide` 决定：`ask` 逐条询问，`edit` 自动放行写文件但 shell 仍询问，`auto` / `guarded` 直接执行，`never` 拒绝需审批的工具。
- `escalate: true` 参数在直接模式下无额外语义（本就不带沙箱）；审批仍按五档执行。
- 系统提示声明：执行根为源项目目录、编辑直接生效、无隔离副本或 worktree、shell 以宿主权限运行、git 命令会真实影响当前分支。
- 右侧「更改审阅」不提供同步动作：只读提示「改动已直接写入源项目目录，无同步审阅」，不请求差异列表、不显示「应用」。直接模式下 `workspace_diff` / `workspace_file_diff` 返回空，`workspace_apply` 拒绝。
- stdio MCP 与生产模式语言服务器启动依赖隔离副本的沙箱租约反推，直接模式与 worktree 模式一致报 `UNSUPPORTED`。
- 无项目会话不受影响，仍使用 scratch root。

## 会话级执行位置

- `SessionInfo` 新增 `execution_mode: Option<ExecutionMode>`；`sessions` 表新增可空列 `execution_mode`（schema 版本 5）。历史行保持 NULL，按旧优先级回退。
- 桌面会话创建命令新增可选 `executionMode` 参数，校验规则：
  - `worktree`：源目录必须含 `.git`，否则 `INVALID_WORKSPACE`；
  - `replica`：隔离副本必须存在，否则 `INVALID_WORKSPACE`（缺少副本的工作区应先切到 `replica` 项目模式触发重建）；
  - `direct`：无额外约束。
- 新建会话草稿态下拉默认值：工作区 `replica` → `replica`；工作区 `direct` → `direct`；工作区旧 `worktree` → `direct`。`replica` 选项仅在工作区模式为 `replica` 时提供；`worktree` 选项仅对 git 仓库提供。
- 会话固化的执行位置不随工作区模式修改而漂移；工作区模式只影响之后新建的会话与历史字段为 NULL 的会话。

## 子代理共享 worktree

- `SessionWorktree` 新增 `capability_id`：登记时写创建者会话 id（与 `shell::prepare` 的 capability 派生一致）。子会话复用父登记时该字段保持父会话 id，shell 提权/沙箱路径使用登记值。
- 新增平台方法 `share_session_worktree(workspace, parent, child)`：把父会话登记复制到子会话键下（相同 path / branch / capability_id），幂等；父未登记时返回空。子代理与 fork 在父会话为 worktree 时调用。

## 各端接入

- 桌面端：创建项目默认直接模式；项目「编辑」弹窗两项；草稿态「执行位置」选择；会话切换时按会话有效模式驱动右侧面板；子代理面板无需变化。
- CLI / JSON-RPC：`--exec-mode` / `workspace.open` 缺省 `direct`；新增显式值可直接创建直接模式工作区。会话创建参数暂不暴露，保持继承工作区默认。
- ACP：`session/new` 继续使用工作区默认模式（不新增参数）。

## 边界与非目标

- 不做 per-turn git 快照或文件 checkpoint/rewind：直接模式没有自动回滚通道，改动的可恢复性依赖用户自身的版本控制与审批策略。后续如需要另行立项。
- 不对源项目目录打沙箱 ACL；直接模式的 shell 不提供受限令牌写边界。
- 不自动清理已登记的会话 worktree；会话删除、归档、工作区模式切换均保留现场。
- 直接模式的审阅面板不展示差异（源项目可能本来就有未提交改动，无法区分归属）；只读提示替代。
- replica 模式的全部现有行为（复制、差异、apply、冲突检测）不变。

## 验收要点

- 新建项目默认直接模式：创建后 `<data>/workspaces/<id>/` 下不产生 `root` 副本目录，源项目目录无新增文件与 DACL 变化。
- 直接模式新建会话并在发送首条消息前后检查：源项目与数据目录均不产生新目录或 worktree。
- 草稿态选择 worktree 后才出现 `.worktrees/harness-<前缀>` 与分支；子代理运行后不新增 worktree。
- 直接模式编辑直落源项目；`ask` 档写文件与 shell 均弹审批，`auto` 档直接执行；右侧面板显示只读提示且无「应用」。
- 旧 worktree 工作区：已登记 worktree 的旧会话继续在原 worktree 执行，新建会话默认直接模式。
- 存量 replica 工作区行为与 UI 完全不变。
