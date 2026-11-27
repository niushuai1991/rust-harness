//! 项目 Hook 审核快照及持久信任记录。
use harness_contracts::{AppError, AppResult};
use harness_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

const KEY: &str = "hooks.project_trust.v1";
static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookReview {
    pub digest: String,
    pub source: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    pub args: Option<Vec<String>>,
    pub script_path: Option<String>,
    pub script_bound: bool,
    pub trusted: bool,
    #[serde(skip)]
    pub(crate) handler: Value,
}

pub struct HookTrustService {
    store: Store,
}
impl HookTrustService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub async fn require_trust(&self, root: &Path) -> AppResult<()> {
        if self.review(root).await?.iter().any(|entry| !entry.trusted) {
            return Err(AppError::new("HOOK_TRUST_REQUIRED", "项目 Hook 需要审核"));
        }
        Ok(())
    }

    pub async fn review(&self, root: &Path) -> AppResult<Vec<HookReview>> {
        let identity = identity(root)?;
        let mut declarations = discover(root)?;
        if declarations.is_empty() {
            return Ok(declarations);
        }
        let _guard = GATE.lock().await;
        let records = self.read().await?;
        for declaration in &mut declarations {
            declaration.trusted = records
                .get(&identity)
                .is_some_and(|list| list.contains(&declaration.digest));
        }
        Ok(declarations)
    }

    pub async fn trust(&self, root: &Path, expected_digest: &str) -> AppResult<()> {
        let _guard = GATE.lock().await;
        let identity = identity(root)?;
        if !discover(root)?
            .iter()
            .any(|entry| entry.digest == expected_digest)
        {
            return Err(error("Hook 已变化或不存在，请重新审核"));
        }
        let mut records = self.read().await?;
        let list = records.entry(identity).or_default();
        if !list.iter().any(|value| value == expected_digest) {
            list.push(expected_digest.to_owned());
        }
        self.store.set(KEY, &json!(records)).await
    }

    pub async fn revoke(&self, root: &Path, digest: &str) -> AppResult<()> {
        let _guard = GATE.lock().await;
        let identity = identity(root)?;
        let mut records = self.read().await?;
        if let Some(list) = records.get_mut(&identity) {
            list.retain(|value| value != digest);
        }
        self.store.set(KEY, &json!(records)).await
    }

    async fn read(&self) -> AppResult<BTreeMap<String, Vec<String>>> {
        match self.store.get(KEY).await? {
            Some(value) => serde_json::from_value(value).map_err(|_| error("Hook 信任记录损坏")),
            None => Ok(BTreeMap::new()),
        }
    }
}

fn error(message: impl std::fmt::Display) -> AppError {
    AppError::new("HOOK_TRUST", message.to_string())
}
fn identity(root: &Path) -> AppResult<String> {
    let path = std::fs::canonicalize(root).map_err(error)?;
    Ok(path.to_string_lossy().into_owned())
}

fn ensure_unlinked(path: &Path) -> AppResult<()> {
    let meta = std::fs::symlink_metadata(path).map_err(error)?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = meta.file_type().is_symlink();
    if linked {
        return Err(error(format!("不允许链接：{}", path.display())));
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> AppResult<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(error)?;
    if file.metadata().map_err(error)?.len() > limit {
        return Err(error("Hook 文件超过大小限制"));
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data).map_err(error)?;
    if data.len() as u64 > limit {
        return Err(error("Hook 文件超过大小限制"));
    }
    Ok(data)
}

fn discover(root: &Path) -> AppResult<Vec<HookReview>> {
    let directory = root.join(".rh");
    match std::fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(self::error(error)),
        Ok(_) => {}
    }
    ensure_unlinked(&directory)?;
    let workspace_identity = identity(root)?;
    let mut result = Vec::new();
    for name in [".settings.json", ".settings-local.json"] {
        let path = directory.join(name);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(self::error(error)),
            Ok(_) => {}
        }
        ensure_unlinked(&path)?;
        let bytes = read_bounded(&path, 1024 * 1024)?;
        let settings: Value = serde_json::from_slice(&bytes).map_err(error)?;
        crate::hooks::validate_value(&settings)?;
        let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
            continue;
        };
        for (event, groups) in events {
            for (group_index, group) in groups.as_array().into_iter().flatten().enumerate() {
                for (hook_index, handler) in
                    group["hooks"].as_array().into_iter().flatten().enumerate()
                {
                    let command = handler["command"]
                        .as_str()
                        .ok_or_else(|| error("Hook 命令缺失"))?;
                    let args = handler.get("args").and_then(Value::as_array).map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    });
                    let script = entry_script(root, command, args.as_deref())?;
                    let script_digest = match &script {
                        Some(path) => {
                            for ancestor in path.ancestors() {
                                ensure_unlinked(ancestor)?;
                            }
                            let content = read_bounded(path, 16 * 1024 * 1024)?;
                            Some(format!("{:x}", Sha256::digest(&content)))
                        }
                        None => None,
                    };
                    let canonical = json!({"version":2,"workspace":workspace_identity,"source":name,"event":event,"group":group_index,"index":hook_index,"matcher":group.get("matcher"),"handler":handler,"script":script.as_ref().map(|p| p.to_string_lossy().into_owned()),"scriptDigest":script_digest});
                    let digest = format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&canonical).map_err(error)?)
                    );
                    result.push(HookReview {
                        digest,
                        source: path.to_string_lossy().into_owned(),
                        event: event.clone(),
                        matcher: group["matcher"].as_str().map(str::to_owned),
                        command: command.to_owned(),
                        args,
                        script_path: script.as_ref().map(|p| p.to_string_lossy().into_owned()),
                        script_bound: script.is_some(),
                        trusted: false,
                        handler: handler.clone(),
                    });
                }
            }
        }
    }
    Ok(result)
}

fn entry_script(root: &Path, command: &str, args: Option<&[String]>) -> AppResult<Option<PathBuf>> {
    let args = args.unwrap_or_default();
    let candidate = args
        .iter()
        .position(|arg| arg.eq_ignore_ascii_case("-file") || arg.eq_ignore_ascii_case("-f"))
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .or_else(|| {
            let name = command.to_ascii_lowercase();
            if [".ps1", ".cmd", ".bat", ".sh", ".py", ".js", ".mjs"]
                .iter()
                .any(|ext| name.ends_with(ext))
            {
                Some(command)
            } else {
                None
            }
        });
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let path = Path::new(candidate);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Ok(Some(path))
}
