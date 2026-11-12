# 项目移除与重开挂回（参考 zcode）

- 日期：2026-11-13（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面侧栏「按项目」视图项目组头右键移除项目（仅移除应用记录，不触碰磁盘原项目）、运行中会话确认停止、项目会话批量转入归档、自动化停用、置顶清理与 MCP/LSP 回收；重开同源路径项目时按移除登记自动挂回旧会话与自动化；`session_restore` 校验项目存在；存档管理对已移除项目显示占位名并禁用恢复。

## 背景与语义

参考 zcode 的项目移除语义（其确认文案）：「该项目还有运行中的对话或 Agent。移除项目会停止并释放相关运行状态，历史任务不会被删除」——有运行中会话时确认后停止而非阻止；无运行中会话时 zcode 直接移除。

zcode 的项目列表是打开的项目标签集合，任务按 workspacePath 关联，重开自动找回；本项目的工作区是持久注册表（`workspaces/<uuid>/record.json`）且会话按 workspace id 关联，项目删除重建后 id 变化、旧会话无法自动挂回，且「最近」视图是跨项目全量会话。因此不能照搬 zcode 的「数据完全不动」：移除用「批量归档 + 移除登记 + 重开挂回」组合实现等价体验（列表即时干净、历史完整保留、重开自动找回）。

## 设计

### 项目移除

- **入口**：「按项目」视图项目组头右键菜单（重命名 / 置顶（取消置顶）/ 移除项目），复用会话行自绘右键菜单的样式与外点/Esc 关闭行为；右键坐标做视口边界收敛。
- **常驻确认弹窗**（比 zcode 关标签更重，移除的是持久记录）：说明项目仅从应用移除（磁盘原项目不受影响）、会话将转入存档管理、绑定自动化停用、重开同目录自动挂回；该项目有运行中会话时追加提示并把确认按钮显示为「移除项目（移除并停止运行）」。
- **命令 `workspace_remove { workspaceId, stopRunning }`**：
  1. 项目下有运行中回合且 `stopRunning=false` 时报 `WORKSPACE_RUNNING`；`stopRunning=true` 时逐会话 cancel 并轮询等待退出（平台层排他锁兜底 BUSY）。
  2. 在归档与停用之前收集登记数据：移除前活跃会话 id、启用的自动化 id、置顶状态。
  3. `Store::archive_workspace_sessions` 批量归档项目下全部未归档会话（记录归档时刻，事件不删；幂等）；`automation_pause_workspace` 停用全部自动化（active 一并转入 paused）。
  4. 清理置顶 KV 残留；写入移除登记（settings KV `workspace.removed.v1`：workspaceId、sourcePath、name、pinned、removedAt、unarchivedSessionIds、automationIds、enabledAutomationIds）。
  5. `Runtime::disconnect_workspace` 断开归属该项目的全部 MCP 连接与 LSP server（单个断开失败不阻塞，对齐 zcode 释放失败不回滚移除的语义）。
  6. `WorkspaceManager::remove`：先校验存在（NOT_FOUND / INVALID_ID），排他锁（运行中的回合与同步持有共享锁，BUSY 拒绝）后删除 `workspaces/<id>/` 记录目录（record.json、隔离副本、备份一并删除），返回被移除的 `WorkspaceInfo`。worktree 执行模式在源项目登记的 `.worktrees/harness-*` 目录与 `feat/session-*` 分支保留不动（属用户仓库，由用户自行清理）。
- **返回 `WorkspaceRemovalSummary { workspaceId, archivedSessions, pausedAutomations, stoppedSessions }`**；前端移除后从项目列表、任务列表、存档集合移除相关条目并清理 localStorage 折叠残留，激活项目被移除时回落到剩余第一个项目或空态，成功提示含归档与停用数量。

### 重开挂回

- **命令 `workspace_reassign_removed { workspaceId }`**：按 `source_path`（canonicalize 后精确匹配）弹出移除登记；无登记返回零值。有登记时：`Store::reassign_sessions` 把旧项目会话批量迁移到新项目——移除前活跃的恢复为未归档（回到任务列表），移除前已归档的保持归档；`automation_reassign_workspace` 迁移自动化 workspaceId 并恢复移除时启用的任务（paused 转 active 并重算下一触发点，错过不补跑），手工暂停的保持暂停；原置顶状态恢复；登记条目消费后移除。
- 打开项目成功后前端自动调用；挂回数量大于零时提示「自动找回 N 个历史会话与 M 个自动化任务」（覆盖创建成功提示）。
- 同源路径重复打开多个项目时挂回触发挂回命令的那个项目；登记 KV 无自动清理（体量小，仅按同 workspaceId 去重覆盖）。

### 存档恢复边界

- `session_restore` 先校验会话所属项目仍存在，已移除报 `WORKSPACE_GONE`（引导重新打开项目自动挂回，而非孤儿恢复）。
- 设置页存档管理按 workspaceId 分组：已移除项目显示占位名「已移除的项目」（沿用 `projectName||占位` 回退），其恢复按钮禁用并提示重新打开项目后会自动挂回。

## 非目标

- 不做会话物理删除与 `message_search` FTS 索引清理（事件与检索保留完整历史）。
- 不做跨路径的项目合并、批量移除或移除撤销（重开同路径即完整找回，登记永不自动清理）。
- CLI / JSON-RPC / ACP 不暴露移除与挂回命令。
- worktree 模式源项目中的会话 worktree 与分支不自动清理。
- 不做重开时对 `worktrees` 登记的迁移（会话 worktree 由会话首次回合懒建）。

## 验证

- `harness-storage`：工作区批量归档只归未归档、幂等、不影响其他工作区（`tests/workspace_removal.rs`）；移除登记 KV 往返与跨重启；会话迁移换绑与选择性恢复归档状态。
- `harness-platform`：remove 删除记录目录保留源项目、未知 id 报错、运行中租约 BUSY、worktree 模式源侧 worktree 保留（`tests/workspaces.rs`）。
- `harness-orchestration`：按工作区停用全部自动化、挂回迁移并恢复启用且重算触发点、手工暂停保持、其他工作区不受影响（`src/tests.rs`）。
- 前端：组头右键菜单、移除确认与列表更新、运行中提示与「移除并停止运行」参数、重开挂回提示、存档管理占位名与禁用恢复（`App.test.tsx`）。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。
