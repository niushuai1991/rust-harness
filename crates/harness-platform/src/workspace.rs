use crate::{diffview, io, paths};
use harness_contracts::{
    AppError, AppResult, FileChange, WorkspaceFileDiff, WorkspaceInfo, new_id, now,
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
        if record.info.id != id || Path::new(&record.info.replica_path) != dir.join("root") {
            return Err(AppError::new("INVALID_WORKSPACE", id));
        }
        // 旧记录（无 name 字段）加载时归一化为目录名，无需迁移即显示正确；下次 save 自然落盘。
        if record.info.name.trim().is_empty() {
            record.info.name = display_name(Path::new(&record.info.source_path), None);
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
    pub fn create(&self, source: &Path, name: Option<String>) -> AppResult<WorkspaceInfo> {
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
        let mut baseline = snapshot(&source)?;
        let id = new_id();
        let dir = self.dir(&id)?;
        let root = dir.join("root");
        fs::create_dir_all(&dir).map_err(io)?;
        File::create(dir.join("sync.lock")).map_err(io)?;
        let _lock = self.lock(&id, true)?;
        let result = (|| {
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
                    .arg(&source)
                    .arg(&root)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "NUL")
                    .output();
                match clone {
                    Ok(out) if out.status.success() => {
                        let has_head = Command::new("git")
                            .arg("-C")
                            .arg(&root)
                            .args(["rev-parse", "--verify", "HEAD"])
                            .env("GIT_CONFIG_NOSYSTEM", "1")
                            .env("GIT_CONFIG_GLOBAL", "NUL")
                            .output()
                            .map_err(io)?
                            .status
                            .success();
                        let output = Command::new("git")
                            .arg("-C")
                            .arg(&root)
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
                            .arg(&root)
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
                            let guard = paths::guarded(&source, path, false)?;
                            if guard.path.is_file() {
                                baseline.insert(path.into(), paths::bytes(&source, path)?);
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir_all(&root).map_err(io)?;
                    }
                    Ok(out) => {
                        return Err(AppError::new("GIT", String::from_utf8_lossy(&out.stderr)));
                    }
                    Err(e) => return Err(io(e)),
                }
            } else {
                fs::create_dir_all(&root).map_err(io)?;
            }
            for (path, data) in &baseline {
                paths::replace(&root, path, data)?;
            }
            crate::shell::prepare(&root, &id)?;
            let info = WorkspaceInfo {
                id: id.clone(),
                source_path: source.to_string_lossy().into_owned(),
                replica_path: root.to_string_lossy().into_owned(),
                created_at: now(),
                name: display_name(&source, name.as_deref()),
                pinned: false,
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
    /// 项目重命名；空白名称拒绝（INVALID_NAME），返回更新后的工作区信息。
    pub fn rename(&self, id: &str, name: String) -> AppResult<WorkspaceInfo> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::new("INVALID_NAME", "名称不能为空"));
        }
        let _lock = self.lock(id, true)?;
        let mut record = self.load(id)?;
        record.info.name = name.to_owned();
        self.save(&record)?;
        Ok(record.info)
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
        Ok(self.changes(&self.load(id)?)?.0)
    }
    /// 单文件 unified diff 视图（桌面端右侧面板懒加载）；未变更的文件返回 NOT_FOUND。
    pub fn diff_view(&self, id: &str, path: &str) -> AppResult<WorkspaceFileDiff> {
        let _lock = self.lock(id, false)?;
        let record = self.load(id)?;
        let change = self
            .changes(&record)?
            .0
            .into_iter()
            .find(|c| c.path == path)
            .ok_or_else(|| AppError::new("NOT_FOUND", path))?;
        Ok(WorkspaceFileDiff {
            path: change.path.clone(),
            unified: diffview::render(&change, false),
        })
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
