use crate::{diffview, io, paths};
use harness_contracts::{
    AppError, AppResult, ExecutionMode, FileChange, SessionWorktree, WorkspaceFileDiff,
    WorkspaceInfo, new_id, now,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
};
type Snapshot = BTreeMap<String, Vec<u8>>;
#[derive(Serialize, Deserialize)]
struct Record {
    info: WorkspaceInfo,
    baseline: Snapshot,
}
#[derive(Clone)]
pub struct WorkspaceManager {
    data_dir: PathBuf,
}
pub struct WorkspaceLease {
    _lock: File,
    pub info: WorkspaceInfo,
}
/// 会话执行目标：执行根、有效执行位置、shell 沙箱 capability 派生 id 与
/// worktree 分支名（直接模式与副本模式无分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionTarget {
    pub root: PathBuf,
    pub mode: ExecutionMode,
    pub capability_id: String,
    pub branch: Option<String>,
}
fn hash(data: Option<&Vec<u8>>) -> Option<String> {
    data.map(|b| hex::encode(Sha256::digest(b)))
}
/// 项目显示名归一化：显式名称去除首尾空白；未填或空白回退为源目录名，永不为空。
fn display_name(source: &Path, name: Option<&str>) -> String {
    if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
        return name.to_owned();
    }
    source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.to_string_lossy().into_owned())
}
fn snapshot(root: &Path) -> AppResult<Snapshot> {
    paths::list_files(root)?
        .into_iter()
        .map(|p| paths::bytes(root, &p).map(|b| (p, b)))
        .collect()
}
/// 准备隔离副本：git 源走无 checkout 克隆并补录追踪文件（含被 ignore 规则覆盖
/// 但仍被 git 跟踪的文件），其他目录直接建空目录；随后把基线内容写入副本并打
/// 沙箱授权。baseline 由调用方先以源目录快照初始化。
fn prepare_replica(id: &str, source: &Path, root: &Path, baseline: &mut Snapshot) -> AppResult<()> {
    if source.join(".git").exists() {
        paths::check(&source.join(".git"))?;
        let clone = Command::new("git")
            .args([
                "-c",
                "core.hooksPath=NUL",
                "clone",
                "--no-local",
                "--no-hardlinks",
                "--no-checkout",
                "--",
            ])
            .arg(source)
            .arg(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output();
        match clone {
            Ok(out) if out.status.success() => {
                let has_head = Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["rev-parse", "--verify", "HEAD"])
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "NUL")
                    .output()
                    .map_err(io)?
                    .status
                    .success();
                let output = Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["read-tree", if has_head { "HEAD" } else { "--empty" }])
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "NUL")
                    .output()
                    .map_err(io)?;
                if !output.status.success() {
                    return Err(AppError::new(
                        "GIT",
                        String::from_utf8_lossy(&output.stderr),
                    ));
                }
                // Git tracks files even when a later ignore rule matches.
                // Query only our fresh replica config, then read original
                // bytes through guarded paths (never run source hooks).
                let tracked = Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["ls-files", "-z"])
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "NUL")
                    .output()
                    .map_err(io)?;
                if !tracked.status.success() {
                    return Err(AppError::new(
                        "GIT",
                        String::from_utf8_lossy(&tracked.stderr),
                    ));
                }
                for raw in tracked.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                    let path = std::str::from_utf8(raw).map_err(io)?;
                    let guard = paths::guarded(source, path, false)?;
                    if guard.path.is_file() {
                        baseline.insert(path.into(), paths::bytes(source, path)?);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(root).map_err(io)?;
            }
            Ok(out) => {
                return Err(AppError::new("GIT", String::from_utf8_lossy(&out.stderr)));
            }
            Err(e) => return Err(io(e)),
        }
    } else {
        fs::create_dir_all(root).map_err(io)?;
    }
    for (path, data) in baseline.iter() {
        paths::replace(root, path, data)?;
    }
    crate::shell::prepare(root, id)?;
    Ok(())
}
fn snapshot_with_baseline(root: &Path, baseline: &Snapshot) -> AppResult<Snapshot> {
    let mut current = snapshot(root)?;
    for path in baseline.keys() {
        let guard = paths::guarded(root, path, false)?;
        if guard.path.try_exists().map_err(io)? {
            current.insert(path.clone(), paths::bytes(root, path)?);
        } else {
            current.remove(path);
        }
    }
    Ok(current)
}
impl WorkspaceManager {
    pub fn new(data_dir: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(&data_dir).map_err(io)?;
        paths::check(&data_dir)?;
        let data_dir = dunce::canonicalize(data_dir).map_err(io)?;
        fs::create_dir_all(data_dir.join("workspaces")).map_err(io)?;
        Ok(Self { data_dir })
    }
    /// 宿主数据目录（截图等浏览器产物存放其下）。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn dir(&self, id: &str) -> AppResult<PathBuf> {
        uuid::Uuid::parse_str(id).map_err(|_| AppError::new("INVALID_ID", id))?;
        Ok(self.data_dir.join("workspaces").join(id))
    }
    fn load(&self, id: &str) -> AppResult<Record> {
        let dir = self.dir(id)?;
        paths::check(&dir)?;
        let mut record: Record =
            serde_json::from_slice(&paths::bytes(&dir, "record.json")?).map_err(io)?;
        if record.info.id != id {
            return Err(AppError::new("INVALID_WORKSPACE", id));
        }
        // replica_path 是派生值：创建时恒为 <数据目录>/workspaces/<id>/root。数据目录
        // 整体搬迁（如旧版 %LOCALAPPDATA%\RustHarness 手工迁移到 ~/.RustHarness）后，
        // 旧记录内嵌的旧位置绝对路径不再匹配，与下方 name 归一化同一模式就地自愈回写，
        // 使搬迁后的工作区仍可加载；自愈值恒为受控路径，篡改无逃逸面。id 不匹配
        // （记录被移入错误目录）仍拒绝。
        let root = dir.join("root");
        if Path::new(&record.info.replica_path) != root {
            record.info.replica_path = root.to_string_lossy().into_owned();
            let _ = self.save(&record);
        }
        // 旧记录（无 name 字段）加载时归一化为目录名并立即回写补齐；回写为原子替换，
        // 失败不阻塞读取（内存归一化已生效），下次加载会再尝试。
        if record.info.name.trim().is_empty() {
            record.info.name = display_name(Path::new(&record.info.source_path), None);
            let _ = self.save(&record);
        }
        Ok(record)
    }
    fn lock(&self, id: &str, exclusive: bool) -> AppResult<File> {
        let dir = self.dir(id)?;
        paths::check(&dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join("sync.lock"))
            .map_err(io)?;
        let result = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        result.map_err(|_| {
            AppError::new("BUSY", "Workspace has an active runtime or synchronization")
        })?;
        Ok(file)
    }
    pub fn acquire(&self, id: &str) -> AppResult<WorkspaceLease> {
        let lock = self.lock(id, false)?;
        Ok(WorkspaceLease {
            _lock: lock,
            info: self.get(id)?,
        })
    }
    pub fn create(
        &self,
        source: &Path,
        name: Option<String>,
        execution_mode: ExecutionMode,
    ) -> AppResult<WorkspaceInfo> {
        paths::check(source)?;
        let source = dunce::canonicalize(source).map_err(io)?;
        if !source.is_dir()
            || source.starts_with(&self.data_dir)
            || self.data_dir.starts_with(&source)
        {
            return Err(AppError::new(
                "INVALID_WORKSPACE",
                "Source and host data must be disjoint directories",
            ));
        }
        if execution_mode == ExecutionMode::Worktree && !source.join(".git").exists() {
            return Err(AppError::new(
                "INVALID_WORKSPACE",
                "worktree execution mode requires a git repository",
            ));
        }
        let mut baseline = if execution_mode == ExecutionMode::Replica {
            snapshot(&source)?
        } else {
            // 直接模式与 worktree 模式都不走副本审阅同步，baseline 恒为空。
            Snapshot::new()
        };
        let id = new_id();
        let dir = self.dir(&id)?;
        let root = dir.join("root");
        fs::create_dir_all(&dir).map_err(io)?;
        File::create(dir.join("sync.lock")).map_err(io)?;
        let _lock = self.lock(&id, true)?;
        let result = (|| {
            if execution_mode != ExecutionMode::Replica {
                // 直接模式以源项目目录为执行根、worktree 模式在 .worktrees/ 下代建，
                // 两者都不建隔离副本；replica_path 仅作记录占位（加载校验依赖其
                // 指向数据目录）。
                let info = WorkspaceInfo {
                    id: id.clone(),
                    source_path: source.to_string_lossy().into_owned(),
                    replica_path: root.to_string_lossy().into_owned(),
                    created_at: now(),
                    name: display_name(&source, name.as_deref()),
                    pinned: false,
                    hidden: false,
                    execution_mode,
                    worktrees: BTreeMap::new(),
                };
                self.save(&Record {
                    info: info.clone(),
                    baseline,
                })?;
                return Ok(info);
            }
            prepare_replica(&id, &source, &root, &mut baseline)?;
            let info = WorkspaceInfo {
                id: id.clone(),
                source_path: source.to_string_lossy().into_owned(),
                replica_path: root.to_string_lossy().into_owned(),
                created_at: now(),
                name: display_name(&source, name.as_deref()),
                pinned: false,
                hidden: false,
                execution_mode,
                worktrees: BTreeMap::new(),
            };
            self.save(&Record {
                info: info.clone(),
                baseline,
            })?;
            Ok(info)
        })();
        if result.is_err() {
            drop(_lock);
            let _ = fs::remove_dir_all(dir);
        }
        result
    }
    fn save(&self, record: &Record) -> AppResult<()> {
        paths::replace(
            &self.dir(&record.info.id)?,
            "record.json",
            &serde_json::to_vec(record).map_err(io)?,
        )
    }
    pub fn list(&self) -> AppResult<Vec<WorkspaceInfo>> {
        let mut infos = Vec::new();
        for entry in fs::read_dir(self.data_dir.join("workspaces")).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if uuid::Uuid::parse_str(&name).is_ok() && entry.path().join("record.json").exists() {
                infos.push(self.get(&name)?);
            }
        }
        infos.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(infos)
    }
    pub fn get(&self, id: &str) -> AppResult<WorkspaceInfo> {
        Ok(self.load(id)?.info)
    }
    /// 编辑项目：名称与执行模式在同一独占锁内一次保存，空白名称拒绝
    /// （INVALID_NAME）。执行模式切换不删除隔离副本、同步基线或已登记的会话
    /// worktree；切到 worktree 要求源目录是 git 仓库（INVALID_WORKSPACE），
    /// 切到 replica 且此前没有副本时从源目录重建。运行中回合持有共享租约时
    /// 返回 BUSY。
    pub fn update(
        &self,
        id: &str,
        name: String,
        execution_mode: ExecutionMode,
    ) -> AppResult<WorkspaceInfo> {
        self.write_update(id, Some(name), Some(execution_mode))
    }
    /// 项目重命名；空白名称拒绝（INVALID_NAME），返回更新后的工作区信息。
    pub fn rename(&self, id: &str, name: String) -> AppResult<WorkspaceInfo> {
        self.write_update(id, Some(name), None)
    }
    fn write_update(
        &self,
        id: &str,
        name: Option<String>,
        execution_mode: Option<ExecutionMode>,
    ) -> AppResult<WorkspaceInfo> {
        let name = name
            .map(|name| {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    Err(AppError::new("INVALID_NAME", "名称不能为空"))
                } else {
                    Ok(name)
                }
            })
            .transpose()?;
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        let mut changed = false;
        if let Some(name) = name
            && record.info.name != name
        {
            record.info.name = name;
            changed = true;
        }
        if let Some(mode) = execution_mode
            && record.info.execution_mode != mode
        {
            match mode {
                ExecutionMode::Worktree => {
                    // 与 create 同口径：只有 git 仓库可切到 worktree。
                    if !Path::new(&record.info.source_path).join(".git").exists() {
                        return Err(AppError::new(
                            "INVALID_WORKSPACE",
                            "worktree execution mode requires a git repository",
                        ));
                    }
                }
                ExecutionMode::Replica => {
                    let root = PathBuf::from(&record.info.replica_path);
                    if !root.exists() {
                        // 直接模式/worktree 模式创建的项目没有隔离副本；切到 replica
                        // 时按源目录当前状态重建副本与基线，失败清理本次新建目录。
                        let source = PathBuf::from(&record.info.source_path);
                        let mut baseline = snapshot(&source)?;
                        let result = prepare_replica(id, &source, &root, &mut baseline);
                        if result.is_err() {
                            let _ = fs::remove_dir_all(&root);
                        }
                        result?;
                        record.baseline = baseline;
                    }
                    // 副本已存在（曾以 replica 运行过）：保留目录与基线，避免
                    // 覆盖尚未同步的改动。
                }
                ExecutionMode::Direct => {
                    // 直接模式无准备动作：保留既有副本与 worktree 登记，切换非破坏性。
                }
            }
            record.info.execution_mode = mode;
            changed = true;
        }
        if changed {
            self.save(&record)?;
        }
        Ok(record.info)
    }
    /// 删除项目记录目录（合并同源重复项目时清理被并入方）：确认无运行时或
    /// 同步占用（独占锁）后整目录删除，隔离副本与备份一并移除。会话、自动化
    /// 与偏好 KV 的迁移由调用方先行完成；记录不存在报 NOT_FOUND，非 UUID 报
    /// INVALID_ID。源项目磁盘目录不受影响；worktree 模式源项目下已代建的会话
    /// worktree 由调用方按返回登记提示用户手动清理。
    pub fn delete(&self, id: &str) -> AppResult<()> {
        let dir = self.dir(id)?;
        if !dir.exists() {
            return Err(AppError::new("NOT_FOUND", id));
        }
        paths::check(&dir)?;
        let _lock = self.lock(id, true)?;
        drop(_lock);
        fs::remove_dir_all(dir).map_err(io)?;
        Ok(())
    }
    /// 宿主为会话代建 worktree（会话选择 worktree 执行位置，首个 turn 前懒调用）。
    ///
    /// 幂等：已登记的会话直接返回。路径为 `<源项目>/.worktrees/harness-<会话 id 前 8 位>`，
    /// 分支 `feat/session-<同前缀>`，基于创建时刻的源项目 HEAD；成功后对 worktree 目录
    /// 打沙箱授权（capability 派生自会话 id）并登记到 record。
    pub fn ensure_session_worktree(
        &self,
        id: &str,
        session_id: &str,
    ) -> AppResult<SessionWorktree> {
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        if let Some(existing) = record.info.worktrees.get(session_id) {
            return Ok(existing.clone());
        }
        if !Path::new(&record.info.source_path).join(".git").exists() {
            return Err(AppError::new(
                "INVALID_WORKSPACE",
                "worktree execution position requires a git repository",
            ));
        }
        let prefix = &session_id[..session_id.len().min(8)];
        let source = PathBuf::from(&record.info.source_path);
        let root = source.join(".worktrees").join(format!("harness-{prefix}"));
        let branch = format!("feat/session-{prefix}");
        fs::create_dir_all(source.join(".worktrees")).map_err(io)?;
        let output = Command::new("git")
            .args(["-c", "core.hooksPath=NUL"])
            .arg("-C")
            .arg(&source)
            .args(["worktree", "add"])
            .arg(&root)
            .args(["-b", &branch, "HEAD"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .map_err(io)?;
        if !output.status.success() {
            return Err(AppError::new(
                "GIT",
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        crate::shell::prepare(&root, session_id)?;
        let worktree = SessionWorktree {
            path: root.to_string_lossy().into_owned(),
            branch,
            capability_id: session_id.to_owned(),
            created_at: now(),
        };
        record
            .info
            .worktrees
            .insert(session_id.to_owned(), worktree.clone());
        self.save(&record)?;
        Ok(worktree)
    }
    /// 子代理/会话 fork 复用父会话 worktree：把父登记复制到子会话键下（同路径、
    /// 同分支、同 capability_id），幂等；父会话未登记时返回 None。
    ///
    /// 调用方必须不持有工作区共享租约（该操作取独占锁写记录）；回合内的
    /// 子代理改用 `execution_target` 的父会话参数解析，不写登记。
    pub fn share_session_worktree(
        &self,
        id: &str,
        parent_session: &str,
        child_session: &str,
    ) -> AppResult<Option<SessionWorktree>> {
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        let Some(parent) = record.info.worktrees.get(parent_session).cloned() else {
            return Ok(None);
        };
        let mut worktree = parent;
        if worktree.capability_id.is_empty() {
            worktree.capability_id = parent_session.to_owned();
        }
        record
            .info
            .worktrees
            .insert(child_session.to_owned(), worktree.clone());
        self.save(&record)?;
        Ok(Some(worktree))
    }

    /// 会话有效执行位置：会话固化值优先；历史会话为空时若已登记 worktree 则沿用
    /// 登记（存量会话不漂移），否则回退工作区默认。
    pub fn resolved_mode(
        &self,
        id: &str,
        session_id: &str,
        session_mode: Option<ExecutionMode>,
    ) -> AppResult<ExecutionMode> {
        let info = self.get(id)?;
        Ok(session_mode
            .or_else(|| {
                info.worktrees
                    .contains_key(session_id)
                    .then_some(ExecutionMode::Worktree)
            })
            .unwrap_or(info.execution_mode))
    }

    /// 解析会话执行根与执行上下文：直接模式为源项目目录（无准备动作）；
    /// replica 为隔离副本；worktree 为已登记的会话 worktree（子代理沿父会话
    /// 登记复用，避免在回合内写记录触发租约冲突），未登记时懒建。
    pub fn execution_target(
        &self,
        id: &str,
        session_id: &str,
        session_mode: Option<ExecutionMode>,
        parent_session: Option<&str>,
    ) -> AppResult<ExecutionTarget> {
        let mode = self.resolved_mode(id, session_id, session_mode)?;
        let info = self.get(id)?;
        match mode {
            ExecutionMode::Direct => Ok(ExecutionTarget {
                root: PathBuf::from(&info.source_path),
                mode,
                capability_id: String::new(),
                branch: None,
            }),
            ExecutionMode::Replica => {
                let root = PathBuf::from(&info.replica_path);
                if !root.is_dir() {
                    return Err(AppError::new(
                        "INVALID_WORKSPACE",
                        "隔离副本不存在；请先将项目执行模式切到 replica",
                    ));
                }
                Ok(ExecutionTarget {
                    root,
                    mode,
                    capability_id: id.to_owned(),
                    branch: None,
                })
            }
            ExecutionMode::Worktree => {
                // 子代理不写自己的登记（回合持有共享租约时写记录会冲突），
                // 直接复用父会话的 worktree 与 capability。
                let registered = info
                    .worktrees
                    .get(session_id)
                    .or_else(|| parent_session.and_then(|parent| info.worktrees.get(parent)));
                let worktree = match registered {
                    Some(existing) => existing.clone(),
                    None => self.ensure_session_worktree(id, session_id)?,
                };
                let capability_id = if worktree.capability_id.is_empty() {
                    session_id.to_owned()
                } else {
                    worktree.capability_id.clone()
                };
                Ok(ExecutionTarget {
                    root: PathBuf::from(&worktree.path),
                    mode,
                    capability_id,
                    branch: Some(worktree.branch),
                })
            }
        }
    }

    /// 会话执行根（解析规则见 [`Self::execution_target`]）。
    pub fn execution_root(
        &self,
        id: &str,
        session_id: &str,
        session_mode: Option<ExecutionMode>,
        parent_session: Option<&str>,
    ) -> AppResult<PathBuf> {
        Ok(self
            .execution_target(id, session_id, session_mode, parent_session)?
            .root)
    }
    /// 无项目会话的执行根：<数据目录>/sessions/<会话 id>/root。懒创建并打沙箱
    /// 授权（capability 派生自会话 id），幂等；会话 id 必须是 UUID，防止路径注入。
    /// scratch root 不注册为工作区记录，也不参与差异审阅与同步。
    pub fn ensure_scratch_root(&self, session_id: &str) -> AppResult<PathBuf> {
        uuid::Uuid::parse_str(session_id).map_err(|_| AppError::new("INVALID_ID", session_id))?;
        let root = self.data_dir.join("sessions").join(session_id).join("root");
        if !root.is_dir() {
            fs::create_dir_all(&root).map_err(io)?;
            crate::shell::prepare(&root, session_id)?;
        }
        Ok(root)
    }

    /// 读取会话 worktree 登记；未登记返回 None。
    pub fn session_worktree(
        &self,
        id: &str,
        session_id: &str,
    ) -> AppResult<Option<SessionWorktree>> {
        Ok(self.get(id)?.worktrees.get(session_id).cloned())
    }
    fn changes(&self, record: &Record) -> AppResult<(Vec<FileChange>, Snapshot, Snapshot)> {
        let after = snapshot_with_baseline(Path::new(&record.info.replica_path), &record.baseline)?;
        let source = snapshot_with_baseline(Path::new(&record.info.source_path), &record.baseline)?;
        let keys: BTreeSet<_> = record.baseline.keys().chain(after.keys()).collect();
        let mut changes = Vec::new();
        for path in keys {
            let before = record.baseline.get(path);
            let next = after.get(path);
            if before == next {
                continue;
            }
            changes.push(FileChange {
                path: path.clone(),
                kind: if before.is_none() {
                    "added"
                } else if next.is_none() {
                    "deleted"
                } else {
                    "modified"
                }
                .into(),
                before_hash: hash(before),
                after_hash: hash(next),
                source_hash: hash(source.get(path)),
                conflict: source.get(path) != before,
                before_text: before.and_then(|b| String::from_utf8(b.clone()).ok()),
                after_text: next.and_then(|b| String::from_utf8(b.clone()).ok()),
            });
        }
        Ok((changes, after, source))
    }
    pub fn diff(&self, id: &str) -> AppResult<Vec<FileChange>> {
        let _lock = self.lock(id, false)?;
        let record = self.load(id)?;
        match record.info.execution_mode {
            ExecutionMode::Worktree => self.worktree_changes(&record),
            // 直接模式改动已落源项目，源项目本身可能带有用户未提交改动，
            // 无法区分归属，不提供差异列表。
            ExecutionMode::Direct => Ok(Vec::new()),
            ExecutionMode::Replica => Ok(self.changes(&record)?.0),
        }
    }
    /// 单文件 unified diff 视图（桌面端右侧面板懒加载）；未变更的文件返回 NOT_FOUND。
    pub fn diff_view(&self, id: &str, path: &str) -> AppResult<WorkspaceFileDiff> {
        let _lock = self.lock(id, false)?;
        let record = self.load(id)?;
        let change = match record.info.execution_mode {
            ExecutionMode::Worktree => self.worktree_changes(&record)?,
            ExecutionMode::Direct => Vec::new(),
            ExecutionMode::Replica => self.changes(&record)?.0,
        }
        .into_iter()
        .find(|c| c.path == path)
        .ok_or_else(|| AppError::new("NOT_FOUND", path))?;
        Ok(WorkspaceFileDiff {
            path: change.path.clone(),
            unified: diffview::render(&change, false),
        })
    }
    /// worktree 模式差异：逐会话 worktree 执行 `git status --porcelain -z` 并汇总
    /// （同路径以先到的会话为准）；无 source_hash/conflict 语义（改动即真实落盘）。
    fn worktree_changes(&self, record: &Record) -> AppResult<Vec<FileChange>> {
        let mut changes = Vec::new();
        let mut seen = BTreeSet::new();
        for worktree in record.info.worktrees.values() {
            let root = PathBuf::from(&worktree.path);
            if !root.is_dir() {
                continue;
            }
            for change in git_worktree_status(&root)? {
                if seen.insert(change.path.clone()) {
                    changes.push(change);
                }
            }
        }
        Ok(changes)
    }
    pub fn apply(&self, id: &str, paths: &[String]) -> AppResult<Vec<FileChange>> {
        self.apply_inner(id, paths, None)
    }
    /// Applies only the exact file hashes the user has reviewed.
    pub fn apply_reviewed(&self, id: &str, reviewed: &[FileChange]) -> AppResult<Vec<FileChange>> {
        let paths = reviewed.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        self.apply_inner(id, &paths, Some(reviewed))
    }
    fn apply_inner(
        &self,
        id: &str,
        paths: &[String],
        reviewed: Option<&[FileChange]>,
    ) -> AppResult<Vec<FileChange>> {
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        if record.info.execution_mode == ExecutionMode::Worktree {
            return Err(AppError::new(
                "APPLY_NOT_SUPPORTED",
                "worktree 模式改动已直接落盘，无需审阅同步",
            ));
        }
        if record.info.execution_mode == ExecutionMode::Direct {
            return Err(AppError::new(
                "APPLY_NOT_SUPPORTED",
                "直接模式改动已写入源项目，无隔离副本可同步",
            ));
        }
        for path in paths {
            paths::validate(path)?;
        }
        let (changes, after, source) = self.changes(&record)?;
        if let Some(reviewed) = reviewed {
            for approved in reviewed {
                let current = changes.iter().find(|c| c.path == approved.path);
                if current.is_none_or(|c| {
                    c.before_hash != approved.before_hash
                        || c.after_hash != approved.after_hash
                        || c.source_hash != approved.source_hash
                        || c.kind != approved.kind
                        || c.conflict != approved.conflict
                }) {
                    return Err(AppError::new("STALE_DIFF", &approved.path));
                }
            }
        }
        let mut selected = Vec::new();
        let mut seen = BTreeSet::new();
        for path in paths {
            if !seen.insert(path) {
                continue;
            }
            let change = changes
                .iter()
                .find(|c| &c.path == path)
                .ok_or_else(|| AppError::new("INVALID_SELECTION", path))?;
            if change.conflict {
                return Err(AppError::new("CONFLICT", path));
            }
            selected.push(change.clone());
        }
        if selected.is_empty() {
            return Ok(selected);
        }
        let backup_dir = self.dir(id)?.join("backups").join(new_id());
        fs::create_dir_all(&backup_dir).map_err(io)?;
        paths::replace(
            &backup_dir,
            "before.json",
            &serde_json::to_vec(&source).map_err(io)?,
        )?;
        paths::replace(
            &backup_dir,
            "selection.json",
            &serde_json::to_vec(&selected).map_err(io)?,
        )?;
        paths::replace(&backup_dir, "status", b"prepared")?;
        let source_path = PathBuf::from(&record.info.source_path);
        let source_root = source_path.as_path();
        let mut committed: Vec<&FileChange> = Vec::new();
        let outcome = (|| {
            for change in &selected {
                let guard = paths::guarded(source_root, &change.path, false)?;
                let current = if guard.path.exists() {
                    Some(paths::bytes(source_root, &change.path)?)
                } else {
                    None
                };
                if current.as_ref() != source.get(&change.path) {
                    return Err(AppError::new("CONFLICT", &change.path));
                }
                if let Some(data) = after.get(&change.path) {
                    paths::replace(source_root, &change.path, data)?;
                } else {
                    fs::remove_file(&guard.path).map_err(io)?;
                }
                committed.push(change);
            }
            for change in &selected {
                if let Some(data) = after.get(&change.path) {
                    record.baseline.insert(change.path.clone(), data.clone());
                } else {
                    record.baseline.remove(&change.path);
                }
            }
            self.save(&record)
        })();
        if let Err(error) = outcome {
            for change in committed.into_iter().rev() {
                let guard = paths::guarded(source_root, &change.path, false)?;
                let current = if guard.path.exists() {
                    Some(paths::bytes(source_root, &change.path)?)
                } else {
                    None
                };
                if current.as_ref() != after.get(&change.path) {
                    return Err(AppError::new(
                        "ROLLBACK_CONFLICT",
                        format!("{}; backup {}", change.path, backup_dir.display()),
                    ));
                }
                if let Some(data) = source.get(&change.path) {
                    paths::replace(source_root, &change.path, data)?;
                } else if guard.path.exists() {
                    fs::remove_file(guard.path).map_err(io)?;
                }
            }
            paths::replace(&backup_dir, "status", b"rolled_back")?;
            return Err(error);
        }
        paths::replace(&backup_dir, "status", b"committed")?;
        Ok(selected)
    }
    pub fn refresh(&self, id: &str) -> AppResult<WorkspaceInfo> {
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        if record.info.execution_mode != ExecutionMode::Replica {
            // worktree/直接模式改动即真实落盘，无副本可同步。
            return Ok(record.info);
        }
        let source = snapshot_with_baseline(Path::new(&record.info.source_path), &record.baseline)?;
        let root = Path::new(&record.info.replica_path);
        for path in paths::list_files(root)? {
            if !source.contains_key(&path) {
                let guard = paths::guarded(root, &path, true)?;
                fs::remove_file(guard.path).map_err(io)?;
            }
        }
        for (path, data) in &source {
            paths::replace(root, path, data)?;
        }
        record.baseline = source;
        self.save(&record)?;
        Ok(record.info)
    }
}

/// 解析单个会话 worktree 的 `git status --porcelain=v1 -z --untracked-files=all` 输出
/// 为 FileChange 列表；rename 条目（`R`/`C`）取目标路径并跳过随后的来源段。
fn git_worktree_status(root: &Path) -> AppResult<Vec<FileChange>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .output()
        .map_err(io)?;
    if !output.status.success() {
        return Err(AppError::new(
            "GIT",
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let mut parts = output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty());
    let mut entries = Vec::new();
    while let Some(raw) = parts.next() {
        let text = std::str::from_utf8(raw).map_err(io)?;
        if text.len() < 4 {
            continue;
        }
        let xy = text[..2].to_owned();
        let path = text[3..].to_owned();
        if xy.contains('R') || xy.contains('C') {
            parts.next();
        }
        entries.push((xy, path));
    }
    entries
        .into_iter()
        .map(|(xy, path)| {
            let kind = if xy == "??" || xy.contains('A') {
                "added"
            } else if xy.contains('D') {
                "deleted"
            } else {
                "modified"
            };
            let before = if kind == "added" {
                None
            } else {
                git_show(root, &path).ok()
            };
            let after = if kind == "deleted" {
                None
            } else {
                Some(paths::bytes(root, &path)?)
            };
            Ok(FileChange {
                path,
                kind: kind.into(),
                before_hash: hash(before.as_ref()),
                after_hash: hash(after.as_ref()),
                source_hash: None,
                conflict: false,
                before_text: before.and_then(|b| String::from_utf8(b).ok()),
                after_text: after
                    .as_ref()
                    .and_then(|b| String::from_utf8(b.clone()).ok()),
            })
        })
        .collect()
}

fn git_show(root: &Path, path: &str) -> AppResult<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("HEAD:{path}")])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .output()
        .map_err(io)?;
    if !output.status.success() {
        return Err(AppError::new(
            "GIT",
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(output.stdout)
}
