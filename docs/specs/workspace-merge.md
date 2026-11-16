# 同源重复项目检测与合并

- 日期：2026-11-17（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面侧栏「按项目」视图检测同源重复项目（同一源目录的多条工作区记录，常见于数据目录手工迁移后重开项目），组头提供「合并同源项目」入口：会话（含归档）、自动化与定时任务引用改挂到保留项目，清理置顶/隐藏偏好残留，删除被并入方的项目记录目录。仅桌面命令，CLI / JSON-RPC / ACP 不暴露。

## 背景与语义

数据目录迁移（默认目录改为 `~/.RustHarness`，不做自动迁移）后重新打开同一源目录会新建项目记录，与迁移而来的旧记录并存：两条记录同源同名，会话分散挂在两个 workspace id 上。「最近」视图跨项目不过滤（会话全量可见），「按项目」视图按 `session.workspaceId` 精确匹配可见项目分组，挂在不匹配项目上的会话不出组，形成「最近比按项目多出会话」的脏数据表现。

合并是数据层的修复动作：与「移除项目」（仅隐藏入口，数据原样保留）相对，合并明确收敛重复记录——被并入方的会话与自动化改挂保留方后，其项目记录目录（含隔离副本与审阅备份）删除。删除是不可逆动作，前端确认弹窗必须说明副本未同步改动随之丢失。

## 设计

### 重复检测

- 前端以 `workspace_list` 的 `sourcePath` 字符串分组，计数大于 1 即视为同源重复；隐藏项目同样参与检测。
- 「按项目」组头在重复项目名称旁显示「重复」标记；组头右键菜单追加「合并同源项目…」（仅重复项目出现）。
- 精确同源性以 `workspace_merge` 命令的 canonicalize 校验为准；前端字符串分组只作入口判断（大小写等差异导致的漏标不影响正确性）。

### 命令 `workspace_merge { fromWorkspaceId, toWorkspaceId, stopRunning }`

1. 校验：双方项目存在且 id 不同；双方 `source_path` canonicalize 后必须一致，否则报 `NOT_DUPLICATE`。
2. `from` 项目有运行中回合且 `stopRunning=false` 时报 `WORKSPACE_RUNNING`；`stopRunning=true` 时逐会话 cancel 并轮询等待退出（与 `workspace_remove` 同模式）。
3. 迁移（顺序执行，失败可重试）：
   - `Store::move_sessions(from, to)`：单条 UPDATE 把全部会话（含归档）改挂目标项目，返回迁移数；事件与 FTS 索引按会话 id 关联，不随迁移变化。
   - `Installed::workspace_retarget(from, to)`：在 `schedules_lock` 内把自动化任务与会话定时任务 KV 中挂在 `from` 的条目改挂 `to`，绑定会话不变，返回（自动化数，定时任务数）。
   - 置顶与隐藏集合移除 `from` 项（记录即将删除）；不改变保留方的置顶/隐藏状态。
4. 删除 `from` 项目记录目录（`WorkspaceManager::delete`）：UUID 校验（`INVALID_ID`）、独占锁确认无运行时或同步占用（占用报 `BUSY`）、记录不存在报 `NOT_FOUND`（合并重试幂等）；整目录删除含隔离副本与审阅备份。源项目磁盘目录不受影响。
5. 返回 `WorkspaceMergeSummary { fromWorkspaceId, toWorkspaceId, movedSessions, movedAutomations, movedSchedules, leftoverWorktrees }`：`leftoverWorktrees` 为 `from` 为 worktree 模式时源项目下仍存在的会话 worktree 路径清单——源项目属用户磁盘资产，宿主不自动删除，仅提示手动清理。

### 前端交互

- 合并弹窗：保留项目下拉（默认当前组项目，展示执行模式与隐藏状态）；说明合并语义、副本未同步改动不可恢复、worktree 残留需手动清理；待并入项目有运行中会话时追加停止警告。
- 多个重复项目时逐个调用 `workspace_merge`（保留方之外全部并入），任一失败即中断并展示错误，已并入项保持合并结果（重试幂等）。
- 合并成功后整体重拉 `workspace_list` 与跨工作区 `session_list`（归属变化跨项目），提示迁移计数与 worktree 残留；激活项目被并入时切换到保留项目。

### 公开契约

- `workspace_merge` 仅桌面命令；CLI / JSON-RPC / ACP 不暴露（与 `workspace_remove` 同级）。
- `WorkspaceManager::delete`、`Store::move_sessions`、`Installed::workspace_retarget` 为内部 API，不进公开契约。

## 非目标

- 不做自动合并或打开目录时的强制提示：重复检测只在组头提供标记与入口，是否合并由用户显式确认。
- 不自动删除源项目下的会话 worktree 目录与分支（见 worktree 执行模式规范）。
- 不合并模型选择、权限覆盖等会话级状态（随会话迁移天然保留）。
- 不处理跨源目录的项目迁移（源目录不同即拒绝）。

## 验证

- `harness-storage`：迁移含归档会话计数、源无会话幂等返回 0、重开数据库归属持久（`tests/session_workspace_merge.rs`）。
- `harness-orchestration`：automation 与 schedule 引用改挂计数、绑定会话不变、其他项目不动。
- `harness-platform`：`delete` 删记录目录、list 不再返回、持锁 BUSY、非法 id 与重复删除错误码（`tests/workspaces.rs`）。
- 前端：重复标记与菜单项条件渲染、无重复无入口、弹窗语义文案、合并参数（from/to/stopRunning）、成功后刷新与提示、运行中警告（`App.test.tsx`）。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。
