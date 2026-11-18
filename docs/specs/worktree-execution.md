# 宿主代建 worktree 执行模式与沙箱提权重跑

- 日期：2026-11-10（计划拟制）；2026-11-18 修订（执行模式支持创建后编辑）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：工作区「worktree 执行模式」——宿主在真实项目 `.worktrees/` 代建 git worktree 并将会话执行根切换过去，文件编辑直落真实分支工作区；shell 沙箱「提权重跑」——命令被沙箱拦截经审批后无沙箱重跑一次。主仓 `.git` 保持对 agent 只读。设计参考 codex（真实 cwd + OS 沙箱 + escalation）与 zcode（`isolation: "worktree"` 契约）。

## 背景与决策

现行架构下桌面与 CLI 会话的执行根一律是隔离副本（`workspaces/<id>/root`），agent 改动只能经 diff 审阅后 `workspace_apply` 同步回真实项目。两个实际问题（2026-11-09「存档按钮」会话复盘）：

1. 会话按仓库 AGENTS.md 约定在**副本内**执行 `git worktree add .worktrees/<任务>`，所有编辑落进副本的 worktree；而同步 baseline 只含 git 管理文件，不含 `.worktrees/`，改动永远无法到达真实项目。
2. 用户明确批准对真实项目执行 `git worktree add` 后，命令仍被受限令牌沙箱拦截（`unable to create directory for .git/refs/heads/...`）——**审批通过不等于写授权**，缺少 codex 式的提权通道。

2026-11-10 用户确认三项决策：

1. **采纳「宿主代建 worktree」方案**（不做「工作区直接模式」，也不做「仅增强同步」）：改动落在真实项目的会话 worktree 中，`main` 工作区永不被 agent 直写。
2. **git 写操作走提权重跑**（抄 codex，不做宿主 git 通道）：commit/push/PR 及 `npm ci` 等一切越界操作统一收敛到「被沙箱拦截 → 审批 → 无沙箱重跑一次」循环。
3. 规范先行：本 spec 批准后再进入实施。

## 执行模式与执行根

- `ExecutionMode` 取值 `direct | replica | worktree`（见[直接执行规范](direct-execution.md)）。`WorkspaceInfo.execution_mode` 表示工作区默认执行位置并落盘 record.json；旧记录缺字段按 `replica` 解释。桌面项目「编辑」弹窗只提供 `direct` / `replica` 两项；`worktree` 改为会话级 opt-in，仅对 git 仓库开放（源目录含 `.git`），新建会话时显式选择。
- `record.json` 新增 `worktrees` 映射：`session_id -> { path, branch, capability_id, created_at }`，登记宿主创建的会话 worktree，供清理与查询；`capability_id` 为创建者会话 id（子会话复用父登记时保持父值）。
- worktree 为**会话级**：新建会话时选择 worktree 才创建，路径 `<sourcePath>/.worktrees/harness-<session_id 前 8 位>/`，分支默认 `feat/session-<session_id 前 8 位>`（宿主无法预知任务性质；模型可在提权批准后 `git branch -m` 改名以符合 AGENTS.md 的 feat/fix 约定）。基于创建时刻的源项目 `HEAD` 新建分支。子代理继承父会话执行位置并沿父登记解析同一 worktree（不写自己的登记）；会话 fork 继承执行位置，在首个 turn 懒建自己的 worktree。
- 创建时机：**会话首个 turn 启动前懒创建**（宿主进程执行 `git worktree add`，不经过 agent 沙箱；`core.hooksPath=NUL` 防源项目 hooks）。创建失败（分支冲突、残留目录等）时本回合报错终止——worktree 模式不建隔离副本，无副本可降级；`.worktrees/` 缺失时先创建该目录。
- 执行根解析收敛为单一来源：`WorkspaceManager::execution_target()` 按「会话固化值 > 已登记 worktree > 工作区默认」返回执行根、执行位置、capability 派生 id 与分支；文件工具、shell、skills/LSP 扫描、`workspace_context`（`harness-runtime/src/agent.rs`）统一消费。
- 系统提示改写（worktree 模式）：「执行根为源项目的 worktree 分支 X（路径 Y），你的编辑直接生效于该分支工作区；worktree 已由宿主创建，无需自行 git worktree add。主仓 `.git` 与 main 工作区不可直接写入，commit/push 等操作用 shell 执行 git 命令，被沙箱拦截时会触发提权审批。」——显式声明代建事实以消解 AGENTS.md「必须自建 worktree」条款与运行环境的指令冲突。

## 执行模式切换（2026-11-18 修订）

- 入口：桌面端项目右键菜单「编辑」弹窗，名称与项目默认执行模式在同一表单内一次保存；平台方法 `WorkspaceManager::update(id, name, execution_mode)` 在独占锁下校验并一次原子写入 `record.json`（`rename` 保留为仅改名的委托）。组头不再提供内联编辑按钮。CLI / JSON-RPC / ACP 不暴露切换。会话级执行位置在新建会话时选择并固化，不在项目弹窗中修改。
- 非破坏性：切到 `direct` 或 `worktree` 时保留隔离副本目录、同步基线与 `worktrees` 登记；副本中尚未同步的改动在切换回 `replica` 后仍可继续审阅。切到 `replica` 时，若副本目录已存在（曾以 replica 运行过）原样保留目录与基线；若此前没有副本（`direct`/`worktree` 创建的项目），按源目录当前快照重建副本与基线。已创建的会话 worktree 目录与分支不删除，会话再次以 worktree 运行时按登记幂等复用。
- 约束：切到 `worktree` 要求源目录是 git 仓库（`INVALID_WORKSPACE`）；桌面 UI 已不再产生项目级 `worktree` 值，该分支仅保留给 CLI / JSON-RPC 兼容与旧记录。名称空白拒绝（`INVALID_NAME`），校验失败时名称与模式均不落盘。运行中回合持共享租约时拒绝（桌面预检返回 `WORKSPACE_RUNNING`，竞态由 `BUSY` 兜底）。
- 生效时机：工作区模式只影响之后新建的会话与历史会话字段为空的会话；已固化的会话执行位置不随工作区设置漂移（解析优先级见[直接执行规范](direct-execution.md)）。
- 计划模式：已处于计划模式且尚未创建 worktree 的会话，切到 worktree 后仍受既有 `PLAN_MODE_WORKTREE` 限制（需退出计划模式完成一次回合，或先以非计划模式运行一次）。

## 沙箱租约与授权

- `sandbox_lease`（`harness-platform/src/shell/windows.rs`）放宽结构校验：执行根允许两类——数据目录副本 root（现状）与已登记工作区的 `<sourcePath>/.worktrees/<dir>`；后者必须与 record.json 的 worktrees 登记匹配，防止路径逃逸。
- capability SID 派生：uuid 形态的 id 直接取字节，其他形态（会话 id）用 uuid5 确定性派生，同一 id 恒得同一 SID。宿主创建 worktree 后调用 `shell::prepare(root, session_id)` 对 worktree 目录打 DACL/完整性标签。ACL 打在**软件自建的目录**上，随目录删除一并消失，无残留。
- 沙箱临时目录沿用 `<data_dir>/workspaces/<wid>/temp`，不落在源项目内。
- worktree 模式下主仓 `.git` 与 main 工作区位于授权树之外，受限令牌天然只读——与 codex 的「保护性元数据」语义等效，无需额外 deny ACE。

## 沙箱提权重跑

- 触发：shell 在沙箱内执行失败（exit_code ≠ 0）且输出命中沙箱拒绝关键词。关键词表收敛在 `harness-platform`（如 `classify_sandbox_denial(output) -> Option<&str>`），内容保守（`permission denied` / `access denied` / `operation not permitted` / `read-only file system` / `unable to create directory` / `cannot lock ref` / `拒绝访问` 等），**宁漏勿误**：漏判走普通审批重试，误判会产生多余审批弹窗。
- 审批：复用现有 `approval.request` 事件，reason 文案「命令疑似被沙箱拦截（<规则>）。是否不带沙箱重跑一次？」，携带原命令。
- 执行：批准后调用新增的 `ShellService::run_unsandboxed(root, command, cancel)`——宿主原令牌直接 spawn，同一 cwd，保留 Job Object 进程树清理与空闲超时；结果入事件流并带 `escalated: true` 标记（回放兼容：旧事件无此字段视为 false）。
- 主动提权：shell 工具参数新增可选 `escalate: true`（语义对齐 codex `require_escalated`），首次即走审批 + 无沙箱执行。
- 与五档权限模式（[权限模式规范](permission-modes.md)）的组合：`decide` 输入增加 `escalated` 维度——`never` → `Some(false)`（拒绝提权）；`ask` / `edit` → `None`（交互裁决）；`auto` → `Some(true)`（完全访问档语义一致）；`guarded` → 危险命令规则命中 `None`，否则 `Some(true)`。矩阵仍唯一实现在 `harness_contracts::ApprovalPolicy`。
- 非目标（v1）：同一 turn 的提权次数上限、提权审计报表。

## 文件工具与审阅面板

- `write` / `edit` 的 read-before-edit 与唯一匹配校验基于执行根当前文件，worktree 模式下即真实 worktree 文件，无需修改逻辑，仅 root 来源切换。
- 右侧「更改审阅」：worktree 模式数据源切换为 `git -C <worktree> diff`（含未跟踪文件清单），只读展示；`workspace_apply` 对 worktree 模式会话禁用（改动已真实落盘）。实现上引入 diff 提供者分流（副本快照对比 / git diff），桌面端 UI 不感知细节，仅隐藏「应用」动作。
- `workspace_refresh` / 状态查询在 worktree 模式返回 worktree 的 git 状态。

## 依赖环境

- worktree 不含被忽略文件（`node_modules`、`target` 等）。v1 **不做 junction/依赖共享**：前端测试前需 `npm ci`、cargo 需全量编译，均通过提权重跑或正常 shell 完成（等待时间长属已知取舍）。
- junction/硬链接共享依赖与构建缓存作为后续优化项，需先解决并发写与清理语义（删除 junction 不得波及目标）。

## 各端接入

- 桌面端：项目「编辑」弹窗提供 `direct` / `replica` 两项，`worktree` 在新建会话草稿态选择（git 仓库可选）；审批卡片复用现有组件，展示提权 reason；会话头或设置处展示执行模式与 worktree 路径。
- CLI：`harness` 工作区创建与 `chat` 的 `--exec-mode` 参数缺省 `direct`；REPL `/status` 展示执行根。
- JSON-RPC / ACP：会话创建沿用工作区默认执行位置；提权审批沿用既有 permission 请求通道。

## 边界与非目标

- 直接模式（执行根=源项目目录）见[直接执行规范](direct-execution.md)；本规范只描述 worktree 与提权重跑。
- 不做宿主 git 通道（一键 commit/PR 面板）：git 写操作统一走提权重跑。
- replica 模式保持全部现有行为：本 spec 全部改动以执行位置分流，replica 路径零变化，回归风险收敛在新增分支。
- 选择 worktree 的会话各自独立 worktree，互不影响；子代理复用父会话 worktree。会话删除不删 worktree（保留现场），工作区删除时按登记表清理宿主创建的 worktree（当前未实现，见[能力状态矩阵](../capability-matrix.md)）。
- v1 已知限制：stdio MCP 服务器与生产模式语言服务器启动依赖隔离副本的沙箱租约反推，worktree 执行模式下明确报错 `UNSUPPORTED`（test seam 注册的 LSP 连接不受影响）。
- 风险：提权关键词误判（保守表缓解；中文系统 PowerShell 错误流已强制 UTF-8 输出并覆盖「的访问被拒绝」等措辞）；AGENTS.md 与系统提示残余冲突（代建声明 + 分支改名通道缓解）；worktree 构建冷启动慢（v1 接受）。

## 验收要点

- worktree 模式会话：编辑直落真实 worktree、`git status` 在真实仓库可见变更、提权审批后完成 commit 与 push、PR 创建成功；主仓 `.git` 直写始终被拦。
- 提权重跑：被拦命令审批后成功执行一次并带 `escalated` 标记；`never` 档拒绝提权；`guarded` 档危险命令仍触发确认。
- 回归：replica 模式工作区的全部现有行为（审阅同步、diff、apply）不变。
- 桌面与 CLI 双端真实抽查。
