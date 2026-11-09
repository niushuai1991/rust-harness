# 工作区（项目）命名规范

- 日期：2026-11-09（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：工作区（项目）显示名的存储、创建时命名、创建后重命名与桌面各显示点；不改变工作副本、同步与审批语义。

## 背景与痛点

项目（工作区）此前没有名称属性：侧栏「任务列表」按项目分组、顶栏面包屑、工作区按钮等显示点一律取 `source_path` 的目录名（前端 `basename` 派生）。用户在打开目录时无法指定名称，同名目录的不同项目也无法区分。

## 设计

1. **存储**：`WorkspaceInfo` 新增非可选字段 `name: String`（serde default），随每工作区 `record.json` 持久化——名称是工作区实体属性而非桌面偏好（与置顶的 settings KV 派生方案不同源），CLI 与桌面共享同一事实源。serde default 保证旧记录可反序列化。
2. **归一化**：创建（`WorkspaceManager::create(source, name)`）时去除首尾空白，未填或空白回退为源目录名（`file_name()`，孤立路径回退全路径）；旧记录（无 name 字段）在 `load()` 时同样归一化为目录名，字段永不为空，无需显式数据迁移——加载补齐时立即回写 `record.json`（`paths::replace` 原子替换；回写失败不阻塞读取，下次加载再试）。
3. **重命名**：`WorkspaceManager::rename(id, name)` 在独占锁下改名并落盘，空白名称拒绝（`INVALID_NAME`）。桌面命令：`workspace_open { path, name? }` 与新增 `workspace_rename { workspaceId, name }`。
4. **桌面入口**：「打开工作区」为两步——选目录后弹命名弹窗（名称预填目录名，留空即用目录名，弹窗内只读展示完整路径）；入口为无激活工作区时欢迎页与侧栏空态的「打开工作区」按钮（2026-11-09 移除侧栏顶部工作区选择按钮后不再有常驻入口，切换既有项目走任务列表）；「按项目」组头改名按钮（hover 浮现，与置顶按钮一致）改名后立即更新。所有显示点（任务列表「按最近」第二行、「按项目」组头与相关 aria-label、面包屑）统一改用 `name` 字段，前端不再做目录名派生回退。

## 非目标

- CLI 不提供命名参数：`harness run --cwd` 隐式创建工作区自动得到目录名。
- 名称不做唯一性约束；不做会话重命名（会话列表规范既有非目标）。

## 验证

- `harness-platform`（`tests/workspaces.rs`）：create 显式命名去空白、未填/空白回退目录名、rename 生效与空名拒绝（INVALID_NAME）、无 name 字段或空白字符串的旧 record.json 加载后归一化且补齐落盘（record.json 中 name 为目录名）、盘符根等无文件名源路径回退全路径（`file_name()`=None 分支）、带空格/中文/含点目录名回退并完成完整创建流程（副本文件可读）、显式名称含中间空格与中文落盘往返一致。
- `frontend/App.test.tsx`：打开工作区弹窗预填目录名并携带名称提交、留空提交传 null（回退目录名并显示）、中文与空格路径预填与提交、组头改名立即显示新名、空名提交显示后端错误且名称不变。
- 提交前运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features` 与 `npm test`。
