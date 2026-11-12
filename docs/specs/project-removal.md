# 项目移除（隐藏入口）与重新显示（参考 zcode）

- 日期：2026-11-13（计划批准，同日修订：移除语义从「删记录 + 归档迁移 + 重开挂回」调整为「仅隐藏入口」，恢复路径放开）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面侧栏「按项目」视图项目组头右键移除项目——仅把项目入口从任务列表隐藏（KV 标记），工作区记录、隔离副本、会话与自动化原样保留，磁盘源项目不受影响；按需停止运行中回合；重开同源路径项目时自动重新显示同目录隐藏项目；归档会话恢复不受隐藏影响。

## 背景与语义

对齐 zcode 的项目移除语义（其文案）：「移除项目会停止并释放相关运行状态，历史任务不会被删除」——移除是入口层的操作，数据不删除、可找回。本项目据此把「移除项目」定为**隐藏语义**：项目记录（`workspaces/<uuid>/` 的 record.json、隔离副本与备份）完整保留，仅通过 settings KV 标记从任务列表入口隐藏；会话（含归档状态）、自动化、置顶均保持原状。因此归档会话的恢复不受任何限制，重新打开同源路径项目时隐藏项目自动重新显示。

## 设计

### 项目移除（隐藏）

- **入口**：「按项目」视图项目组头右键菜单（重命名 / 置顶（取消置顶）/ 移除项目），复用会话行自绘右键菜单的样式与外点/Esc 关闭行为；右键坐标做视口边界收敛。
- **常驻确认弹窗**：说明项目入口将从任务列表隐藏，磁盘原项目、会话与自动化数据全部保留；该项目有运行中会话时追加提示并把确认按钮显示为「移除项目（移除并停止运行）」。
- **命令 `workspace_remove { workspaceId, stopRunning }`**：
  1. 项目下有运行中回合且 `stopRunning=false` 时报 `WORKSPACE_RUNNING`；`stopRunning=true` 时逐会话 cancel 并轮询等待退出。
  2. 写隐藏标记（settings KV `workspace.hidden.v1`，与置顶 `workspace.pinned.v1` 同模式）。
  3. 不删工作区记录、不归档会话、不停用自动化、不清置顶、不断开 MCP/LSP——数据与运行环境原样保留。
- **返回 `WorkspaceRemovalSummary { workspaceId, stoppedSessions }`**；前端本地把该项目的 `hidden` 置位（不重新拉取），激活项目被移除时回落到其余可见项目或空态，提示「数据全部保留；重新打开同一目录的项目会重新显示它」。

### 隐藏的显示边界

- **「按项目」视图**：隐藏项目不出组（前端以 `WorkspaceInfo.hidden` 过滤后传给 `groupByProject`）；**草稿态项目候选**（chip 菜单）同样过滤。
- **「最近」视图**：隐藏项目的会话不过滤——数据保留且随时可达（记录在，点击打开、工作副本、差异审阅等功能完整可用），这是「历史不删除」的直接体现。
- **存档管理**：隐藏项目的归档会话照常按项目分组显示（显示真实项目名），恢复按钮不受隐藏影响。
- **公开契约**：`WorkspaceInfo` 追加 `hidden: bool`（serde default，camelCase），与 `pinned` 同为桌面命令层从 KV 派生注入；CLI / JSON-RPC 契约不受影响。

### 重新显示（重开同源路径）

- **命令 `workspace_reveal_hidden { workspaceId }`**：打开目录创建项目后调用，按 `source_path`（canonicalize 精确匹配）找出同目录的全部隐藏项目并取消隐藏（移出集合）；返回 `WorkspaceRevealSummary { workspaceId, revealed }`。
- 前端在 `workspace_open` 成功后自动调用；`revealed > 0` 时提示「重新显示 N 个同目录隐藏项目（其会话与自动化原位保留）」。会话与自动化从不迁移——项目记录未动，无需挂回。

### 存档恢复

- `session_restore` 保持既有语义（仅校验会话存在且已归档），不校验项目是否隐藏——隐藏项目的归档会话随时可恢复。

## 非目标

- 不做会话物理删除与 FTS 索引清理；不做隐藏项目的彻底删除入口（后续可在设置页补充）。
- 隐藏项目的自动化任务继续按计划执行（数据不动语义）；不做隐藏时自动暂停。
- CLI / JSON-RPC / ACP 不暴露隐藏与重新显示命令。
- worktree 模式源项目中的会话 worktree 与分支管理不在本规范范围（见 worktree 执行模式规范）。

## 验证

- `harness-storage`：隐藏集合 KV 往返、去重、跨重启；隐藏不影响会话查询与归档恢复（`tests/workspace_removal.rs`）。
- `harness-contracts`：`WorkspaceInfo` serde 缺省字段回归（`hidden` 缺省 false，与 `pinned` 同模式）。
- 前端：组头右键菜单、隐藏确认与列表更新（组消失、其他显示点保留）、运行中提示与「移除并停止运行」参数、重开重新显示提示、存档管理对隐藏项目可查看可恢复（`App.test.tsx`）。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。
