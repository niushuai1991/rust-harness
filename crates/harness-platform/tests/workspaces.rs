use harness_contracts::ExecutionMode;
use harness_platform::*;
use std::{fs, path::PathBuf};
fn fixture() -> (tempfile::TempDir, PathBuf, WorkspaceManager) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
    // 基址先归一化（去掉 `..`），与 WorkspaceManager::new 对数据目录的
    // canonicalize 保持一致，避免 temp.path() 拼出的期望路径带 `..` 段。
    let base = dunce::canonicalize(&base).unwrap();
    let temp = tempfile::tempdir_in(base).unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("a.txt"), "initial").unwrap();
    let manager = WorkspaceManager::new(temp.path().join("host")).unwrap();
    (temp, source, manager)
}
#[test]
fn changes_require_confirmation_and_detect_external_conflicts() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    write_file(std::path::Path::new(&info.replica_path), "a.txt", "edited").unwrap();
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "initial");
    assert_eq!(manager.diff(&info.id).unwrap().len(), 1);
    assert!(manager.apply(&info.id, &[]).unwrap().is_empty());
    fs::write(source.join("a.txt"), "external").unwrap();
    assert!(manager.diff(&info.id).unwrap()[0].conflict);
    assert!(manager.apply(&info.id, &["a.txt".into()]).is_err());
    assert_eq!(
        fs::read_to_string(source.join("a.txt")).unwrap(),
        "external"
    );
}
#[test]
fn leases_prevent_sync_across_manager_instances() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let lease = manager.acquire(&info.id).unwrap();
    let other = WorkspaceManager::new(source.parent().unwrap().join("host")).unwrap();
    assert_eq!(other.refresh(&info.id).unwrap_err().code, "BUSY");
    assert_eq!(manager.apply(&info.id, &[]).unwrap_err().code, "BUSY");
    drop(lease);
    manager.refresh(&info.id).unwrap();
}
#[test]
fn confirmed_apply_updates_baseline_and_persists() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    write_file(std::path::Path::new(&info.replica_path), "a.txt", "edited").unwrap();
    manager.apply(&info.id, &["a.txt".into()]).unwrap();
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "edited");
    assert!(manager.diff(&info.id).unwrap().is_empty());
    assert_eq!(manager.list().unwrap().len(), 1);
}
#[test]
fn rejects_escape_ads_and_nested_data() {
    let (_temp, source, manager) = fixture();
    for path in [
        "../outside",
        "a:stream",
        "C:/outside",
        "a/../b",
        "a\\..\\b",
        ".git/config",
        "CON",
        "x. ",
    ] {
        assert!(resolve_path(&source, path, false).is_err(), "{path}");
    }
    let nested = WorkspaceManager::new(source.join("host")).unwrap();
    assert!(
        nested
            .create(&source, None, ExecutionMode::Replica)
            .is_err()
    );
    let _ = manager;
}

#[test]
fn ignored_caches_are_not_copied_and_hardlinks_are_rejected() {
    let (_temp, source, manager) = fixture();
    fs::write(source.join(".gitignore"), "cache/\n").unwrap();
    fs::create_dir(source.join("cache")).unwrap();
    fs::write(source.join("cache/large"), "ignored").unwrap();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    assert!(
        !std::path::Path::new(&info.replica_path)
            .join("cache")
            .exists()
    );
    fs::hard_link(source.join("a.txt"), source.join("linked.txt")).unwrap();
    assert_eq!(
        manager
            .create(&source, None, ExecutionMode::Replica)
            .unwrap_err()
            .code,
        "UNSAFE_LINK"
    );
}

#[test]
fn git_copy_preserves_history_without_sharing_metadata() {
    use std::process::Command;
    let (_temp, source, manager) = fixture();
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    for args in [
        vec!["init"],
        vec!["add", "a.txt"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::write(source.join("a.txt"), "dirty").unwrap();
    fs::write(source.join(".gitignore"), "a.txt\n").unwrap();
    fs::write(source.join("untracked.txt"), "new").unwrap();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let root = std::path::Path::new(&info.replica_path);
    assert_eq!(read_file(root, "a.txt").unwrap(), "dirty");
    assert_eq!(read_file(root, "untracked.txt").unwrap(), "new");
    assert!(root.join(".git/objects").is_dir());
    assert!(!root.join(".git/objects/info/alternates").exists());
    assert!(manager.diff(&info.id).unwrap().is_empty());
}

#[test]
fn empty_git_project_can_be_imported() {
    let (_temp, source, manager) = fixture();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&source)
            .arg("init")
            .output()
            .unwrap()
            .status
            .success()
    );
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    assert_eq!(
        read_file(std::path::Path::new(&info.replica_path), "a.txt").unwrap(),
        "initial"
    );
}

#[test]
fn new_ignore_rule_does_not_turn_baseline_file_into_a_deletion() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    write_file(
        std::path::Path::new(&info.replica_path),
        ".gitignore",
        "a.txt\n",
    )
    .unwrap();
    let changes = manager.diff(&info.id).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, ".gitignore");
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "initial");
}

#[test]
fn apply_reviewed_refuses_replica_or_source_changes_after_preview() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let root = std::path::Path::new(&info.replica_path);
    write_file(root, "a.txt", "reviewed v1").unwrap();
    let reviewed = manager.diff(&info.id).unwrap();
    write_file(root, "a.txt", "unreviewed v2").unwrap();
    assert_eq!(
        manager
            .apply_reviewed(&info.id, &reviewed)
            .unwrap_err()
            .code,
        "STALE_DIFF"
    );
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "initial");
    write_file(root, "a.txt", "reviewed v1").unwrap();
    fs::write(source.join("a.txt"), "external").unwrap();
    assert_eq!(
        manager
            .apply_reviewed(&info.id, &reviewed)
            .unwrap_err()
            .code,
        "STALE_DIFF"
    );
    fs::write(source.join("a.txt"), "initial").unwrap();
    manager.apply_reviewed(&info.id, &reviewed).unwrap();
    assert_eq!(
        fs::read_to_string(source.join("a.txt")).unwrap(),
        "reviewed v1"
    );
}

#[cfg(windows)]
#[test]
fn junction_components_are_rejected_before_host_operations() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let root = std::path::Path::new(&info.replica_path);
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(root.join("junction"))
        .arg(dunce::canonicalize(&source).unwrap())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(write_file(root, "junction/a.txt", "escape").is_err());
    assert!(manager.diff(&info.id).is_err());
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "initial");
    fs::remove_dir(root.join("junction")).unwrap();
}

#[test]
fn diff_view_renders_unified_diff_per_file() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let replica = std::path::Path::new(&info.replica_path);
    write_file(
        replica,
        "a.txt",
        "edited
more
",
    )
    .unwrap();
    write_file(
        replica,
        "b.txt",
        "new file
",
    )
    .unwrap();
    // modified：unified diff 带文件头与增删标记
    let view = manager.diff_view(&info.id, "a.txt").unwrap();
    assert_eq!(view.path, "a.txt");
    let unified = view.unified.unwrap();
    assert!(unified.contains("--- a.txt"));
    assert!(unified.contains("+edited"));
    // added：before 侧视为空
    let view = manager.diff_view(&info.id, "b.txt").unwrap();
    assert!(view.unified.unwrap().contains("+new file"));
    // 二进制：unified 为 None
    fs::write(replica.join("bin.dat"), [0xFFu8, 0x00, 0x0A]).unwrap();
    let view = manager.diff_view(&info.id, "bin.dat").unwrap();
    assert!(view.unified.is_none());
    // 副本未动过的文件不产生改动视图
    assert_eq!(
        manager.diff_view(&info.id, "missing.txt").unwrap_err().code,
        "NOT_FOUND"
    );
}

#[test]
fn create_names_workspace_or_falls_back_to_directory_name() {
    let (_temp, source, manager) = fixture();
    // 显式命名：去除首尾空白后落库
    let named = manager
        .create(&source, Some("  我的项目  ".into()), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(named.name, "我的项目");
    // 未填名称（None 或空白）回退为源目录名
    let blank = manager
        .create(&source, Some("   ".into()), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(blank.name, "source");
    let none = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    assert_eq!(none.name, "source");
    // 持久化后重读仍一致
    assert_eq!(manager.get(&named.id).unwrap().name, "我的项目");
}

#[test]
fn rename_updates_name_and_rejects_blank() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let renamed = manager.rename(&info.id, "新名字".into()).unwrap();
    assert_eq!(renamed.name, "新名字");
    assert_eq!(manager.get(&info.id).unwrap().name, "新名字");
    // 其他字段不受影响
    assert_eq!(manager.get(&info.id).unwrap().source_path, info.source_path);
    assert_eq!(
        manager.rename(&info.id, "   ".into()).unwrap_err().code,
        "INVALID_NAME"
    );
    assert_eq!(manager.get(&info.id).unwrap().name, "新名字");
}

#[test]
fn legacy_records_without_name_fall_back_to_directory_name() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    // 手工把 record.json 改写为无 name 字段的旧格式，加载时应归一化为目录名
    let record_path = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .join("record.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    value["info"]
        .as_object_mut()
        .unwrap()
        .remove("name")
        .expect("新建记录应已包含 name 字段");
    let value = serde_json::to_vec(&value).unwrap();
    fs::write(&record_path, value).unwrap();
    assert_eq!(manager.get(&info.id).unwrap().name, "source");
    assert_eq!(manager.list().unwrap()[0].name, "source");
    // 补齐即落盘：record.json 已写入目录名，后续无需依赖加载期归一化
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(persisted["info"]["name"], "source");
}

#[test]
fn create_falls_back_to_directory_name_for_special_paths() {
    let (temp, _source, manager) = fixture();
    // 目录名回退需保留空格、中文与点号；同时验证特殊路径下完整创建流程（副本文件可读）
    for name in ["my dir", "我的项目", "我的 项目", "v1.2"] {
        let source = temp.path().join(name);
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a.txt"), "content").unwrap();
        let info = manager
            .create(&source, None, ExecutionMode::Replica)
            .unwrap();
        assert_eq!(info.name, name, "目录名 {name} 应原样回退为显示名");
        assert_eq!(
            fs::read_to_string(std::path::Path::new(&info.replica_path).join("a.txt")).unwrap(),
            "content"
        );
        assert_eq!(manager.get(&info.id).unwrap().name, name);
    }
}

#[test]
fn blank_stored_name_is_normalized_on_load() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let record_path = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .join("record.json");
    // 与字段缺失不同的触发路径：name 键存在但为空白字符串
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    value["info"]["name"] = serde_json::Value::String("   ".into());
    fs::write(&record_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(manager.get(&info.id).unwrap().name, "source");
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(persisted["info"]["name"], "source");
}

#[test]
fn relocated_host_dir_heals_stale_replica_path() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    // 模拟数据目录整体搬迁（如 %LOCALAPPDATA%\RustHarness 手工迁移到 ~/.RustHarness）：
    // record.json 内嵌的 replicaPath 仍指向旧位置绝对路径
    let record_path = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .join("record.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    let stale = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("RustHarness-old")
        .join(&info.id)
        .join("root");
    value["info"]["replicaPath"] = serde_json::Value::String(stale.to_string_lossy().into_owned());
    fs::write(&record_path, serde_json::to_vec(&value).unwrap()).unwrap();
    // 加载应自愈为当前数据目录下的真实位置，而不是 INVALID_WORKSPACE
    let healed = manager.get(&info.id).unwrap();
    assert_eq!(healed.replica_path, info.replica_path);
    assert_eq!(manager.list().unwrap()[0].replica_path, info.replica_path);
    // 自愈即落盘：record.json 已回写，后续加载无需再修
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(persisted["info"]["replicaPath"], info.replica_path);
    // 自愈后副本差异审阅全链路可用
    write_file(std::path::Path::new(&info.replica_path), "a.txt", "edited").unwrap();
    assert_eq!(manager.diff(&info.id).unwrap().len(), 1);
}

#[test]
fn root_source_path_falls_back_to_full_path() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let record_path = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .join("record.json");
    // 纯字符串构造 file_name()=None 的源路径（盘符根），覆盖全路径回退分支；不触碰真实根目录
    // 注意 record.json 的键是 camelCase（serde rename_all），蛇形键会被 serde 静默忽略
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    value["info"]["sourcePath"] = serde_json::Value::String("D:/".into());
    value["info"].as_object_mut().unwrap().remove("name");
    fs::write(&record_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(manager.get(&info.id).unwrap().name, "D:/");
}

#[test]
fn explicit_name_with_spaces_and_unicode_roundtrips() {
    let (_temp, source, manager) = fixture();
    // trim 只去首尾空白，中间空格与中文保留；创建与重命名都需落盘往返一致
    let info = manager
        .create(&source, Some("My 项目 α".into()), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(info.name, "My 项目 α");
    assert_eq!(manager.get(&info.id).unwrap().name, "My 项目 α");
    let renamed = manager.rename(&info.id, "我的 项目".into()).unwrap();
    assert_eq!(renamed.name, "我的 项目");
    assert_eq!(manager.get(&info.id).unwrap().name, "我的 项目");
}

#[test]
fn diff_view_still_renders_when_source_side_conflicts() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    write_file(std::path::Path::new(&info.replica_path), "a.txt", "edited").unwrap();
    fs::write(source.join("a.txt"), "external").unwrap();
    let changes = manager.diff(&info.id).unwrap();
    assert!(changes[0].conflict);
    // 冲突文件渲染的是基线与副本的差异，apply 由既有哈希校验兜底。
    let view = manager.diff_view(&info.id, "a.txt").unwrap();
    assert!(view.unified.unwrap().contains("+edited"));
}

fn git_commit_source(source: &std::path::Path) {
    use std::process::Command;
    for args in [
        vec!["init", "-q"],
        vec!["add", "a.txt"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(source)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn worktree_mode_requires_git_repository() {
    let (_temp, source, manager) = fixture();
    assert_eq!(
        manager
            .create(&source, None, ExecutionMode::Worktree)
            .unwrap_err()
            .code,
        "INVALID_WORKSPACE"
    );
}

#[test]
fn worktree_mode_creates_and_registers_session_worktree() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    assert_eq!(info.execution_mode, ExecutionMode::Worktree);
    // worktree 模式不创建隔离副本目录，replica_path 仅为记录占位。
    assert!(!std::path::Path::new(&info.replica_path).exists());

    let worktree = manager
        .ensure_session_worktree(&info.id, "sess1a2b")
        .unwrap();
    let expected = PathBuf::from(&info.source_path)
        .join(".worktrees")
        .join("harness-sess1a2b");
    assert_eq!(worktree.path, expected.to_string_lossy());
    assert_eq!(worktree.branch, "feat/session-sess1a2b");
    assert!(expected.join(".git").exists());
    assert_eq!(read_file(&expected, "a.txt").unwrap(), "initial");

    // 幂等：重复调用返回同一登记。
    let again = manager
        .ensure_session_worktree(&info.id, "sess1a2b")
        .unwrap();
    assert_eq!(again, worktree);

    let reloaded = manager.get(&info.id).unwrap();
    assert_eq!(reloaded.worktrees["sess1a2b"], worktree);
    assert_eq!(
        manager.execution_root(&info.id, "sess1a2b").unwrap(),
        expected
    );
    // 其他会话懒建各自的 worktree。
    let other = manager.execution_root(&info.id, "sess3c4d").unwrap();
    assert_ne!(other, expected);
    assert!(other.join(".git").exists());
}

#[test]
fn worktree_mode_diff_tracks_real_changes_without_source_hash() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let worktree = manager.execution_root(&info.id, "sess1a2b").unwrap();
    fs::write(worktree.join("a.txt"), "edited").unwrap();
    fs::write(worktree.join("new.txt"), "added").unwrap();
    let changes = manager.diff(&info.id).unwrap();
    let modified = changes.iter().find(|c| c.path == "a.txt").unwrap();
    assert_eq!(modified.kind, "modified");
    assert_eq!(modified.before_text.as_deref(), Some("initial"));
    assert_eq!(modified.after_text.as_deref(), Some("edited"));
    assert!(!modified.conflict);
    let added = changes.iter().find(|c| c.path == "new.txt").unwrap();
    assert_eq!(added.kind, "added");
    assert_eq!(added.before_text, None);
    // 删除真实文件后不再出现在差异中。
    fs::remove_file(worktree.join("new.txt")).unwrap();
    let changes = manager.diff(&info.id).unwrap();
    assert!(changes.iter().all(|c| c.path != "new.txt"));
}

#[test]
fn worktree_mode_apply_is_rejected_and_refresh_is_noop() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let worktree = manager.execution_root(&info.id, "sess1a2b").unwrap();
    fs::write(worktree.join("a.txt"), "edited").unwrap();
    assert_eq!(
        manager.apply(&info.id, &["a.txt".into()]).unwrap_err().code,
        "APPLY_NOT_SUPPORTED"
    );
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "initial");
    manager.refresh(&info.id).unwrap();
    assert_eq!(
        fs::read_to_string(worktree.join("a.txt")).unwrap(),
        "edited"
    );
}

#[test]
fn worktree_mode_diff_view_renders_unified() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let worktree = manager.execution_root(&info.id, "sess1a2b").unwrap();
    fs::write(worktree.join("a.txt"), "edited").unwrap();
    let view = manager.diff_view(&info.id, "a.txt").unwrap();
    let unified = view.unified.unwrap();
    assert!(unified.contains("+edited"), "{unified}");
    assert!(unified.contains("-initial"), "{unified}");
}

#[test]
fn delete_removes_record_directory_and_rejects_invalid_ids() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let dir = std::path::Path::new(&info.replica_path)
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(dir.join("record.json").exists());

    manager.delete(&info.id).unwrap();
    assert!(!dir.exists(), "记录目录应整目录删除");
    assert!(manager.list().unwrap().is_empty());
    assert!(manager.get(&info.id).is_err());
    // 非 UUID id 拒绝；删除后重复删除报 NOT_FOUND（合并重试场景）。
    assert_eq!(manager.delete("not-a-uuid").unwrap_err().code, "INVALID_ID");
    assert_eq!(manager.delete(&info.id).unwrap_err().code, "NOT_FOUND");
}

#[test]
fn delete_rejects_while_runtime_lease_held() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let lease = manager.acquire(&info.id).unwrap();
    assert_eq!(manager.delete(&info.id).unwrap_err().code, "BUSY");
    drop(lease);
    manager.delete(&info.id).unwrap();
    assert!(manager.list().unwrap().is_empty());
}

/// 无项目会话执行根：按会话在数据目录下懒建独立 scratch root，幂等且拒绝非法
/// 会话 id；不注册为工作区记录，不参与差异审阅与同步。
#[test]
fn scratch_roots_are_lazy_unique_and_idempotent() {
    let (temp, _source, manager) = fixture();
    let sid = "018f0e5b-1111-2222-3333-444455556666";
    let root = manager.ensure_scratch_root(sid).unwrap();
    let expected = temp
        .path()
        .join("host")
        .join("sessions")
        .join(sid)
        .join("root");
    assert_eq!(root, expected);
    assert!(expected.is_dir());

    // 幂等：重复调用返回同一目录且保留既有文件。
    fs::write(expected.join("keep.txt"), "data").unwrap();
    let again = manager.ensure_scratch_root(sid).unwrap();
    assert_eq!(again, expected);
    assert_eq!(
        fs::read_to_string(expected.join("keep.txt")).unwrap(),
        "data"
    );

    // 每个会话独立 root。
    let other = manager
        .ensure_scratch_root("018f0e5b-aaaa-bbbb-cccc-dddd55556666")
        .unwrap();
    assert_ne!(other, expected);

    // 非法 id 拒绝（路径注入防线）。
    assert_eq!(
        manager.ensure_scratch_root("../escape").unwrap_err().code,
        "INVALID_ID"
    );
    assert_eq!(
        manager.ensure_scratch_root("not-a-uuid").unwrap_err().code,
        "INVALID_ID"
    );

    // scratch root 不注册为工作区记录。
    assert!(manager.list().unwrap().is_empty());
}

#[test]
fn update_renames_and_switches_mode_in_one_operation() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let updated = manager
        .update(&info.id, "新名字".into(), ExecutionMode::Worktree)
        .unwrap();
    assert_eq!(updated.name, "新名字");
    assert_eq!(updated.execution_mode, ExecutionMode::Worktree);
    let reloaded = manager.get(&info.id).unwrap();
    assert_eq!(reloaded.name, "新名字");
    assert_eq!(reloaded.execution_mode, ExecutionMode::Worktree);
    // 切到 worktree 不删除隔离副本。
    assert!(
        std::path::Path::new(&info.replica_path)
            .join("a.txt")
            .is_file()
    );
}

#[test]
fn update_rejects_blank_name_without_switching_mode() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    assert_eq!(
        manager
            .update(&info.id, "   ".into(), ExecutionMode::Worktree)
            .unwrap_err()
            .code,
        "INVALID_NAME"
    );
    let unchanged = manager.get(&info.id).unwrap();
    assert_eq!(unchanged.name, info.name);
    assert_eq!(unchanged.execution_mode, ExecutionMode::Replica);
}

#[test]
fn update_rejects_worktree_switch_on_non_git_source_without_partial_write() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    assert_eq!(
        manager
            .update(&info.id, "新名字".into(), ExecutionMode::Worktree)
            .unwrap_err()
            .code,
        "INVALID_WORKSPACE"
    );
    // 名字与模式均不落盘（先在内存完成校验，失败不保存）。
    let unchanged = manager.get(&info.id).unwrap();
    assert_eq!(unchanged.name, info.name);
    assert_eq!(unchanged.execution_mode, ExecutionMode::Replica);
}

#[test]
fn switching_to_worktree_preserves_replica_changes_for_switch_back() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let root = std::path::Path::new(&info.replica_path);
    write_file(root, "a.txt", "edited").unwrap();
    manager
        .update(&info.id, info.name.clone(), ExecutionMode::Worktree)
        .unwrap();
    // worktree 模式差异走 git status，不展示副本改动，但副本内容原样保留。
    assert!(manager.diff(&info.id).unwrap().is_empty());
    assert_eq!(read_file(root, "a.txt").unwrap(), "edited");
    // 切回 replica 后未同步改动仍可审阅。
    let back = manager
        .update(&info.id, info.name.clone(), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(back.execution_mode, ExecutionMode::Replica);
    let changes = manager.diff(&info.id).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "a.txt");
    assert!(!changes[0].conflict);
}

#[test]
fn switching_to_replica_rebuilds_replica_from_source() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    // worktree 模式创建的项目没有隔离副本。
    assert!(!std::path::Path::new(&info.replica_path).exists());
    let root = std::path::PathBuf::from(&info.replica_path);
    let updated = manager
        .update(&info.id, info.name.clone(), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(updated.execution_mode, ExecutionMode::Replica);
    assert_eq!(read_file(&root, "a.txt").unwrap(), "initial");
    // 基线为源目录快照：未改动时无差异；切回后审阅同步恢复可用。
    assert!(manager.diff(&info.id).unwrap().is_empty());
    write_file(&root, "a.txt", "edited").unwrap();
    manager.apply(&info.id, &["a.txt".into()]).unwrap();
    assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "edited");
}

#[test]
fn switching_back_to_worktree_reuses_registered_session_worktree() {
    let (_temp, source, manager) = fixture();
    git_commit_source(&source);
    let info = manager
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let worktree = manager
        .ensure_session_worktree(&info.id, "sess1a2b")
        .unwrap();
    manager
        .update(&info.id, info.name.clone(), ExecutionMode::Replica)
        .unwrap();
    // 切到 replica 不清理登记：已有 worktree 路径保留。
    assert_eq!(
        manager.get(&info.id).unwrap().worktrees["sess1a2b"].path,
        worktree.path
    );
    manager
        .update(&info.id, info.name.clone(), ExecutionMode::Worktree)
        .unwrap();
    assert_eq!(
        manager.execution_root(&info.id, "sess1a2b").unwrap(),
        PathBuf::from(&worktree.path)
    );
}

#[test]
fn update_rejects_while_lease_held_and_noop_is_stable() {
    let (_temp, source, manager) = fixture();
    let info = manager
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let lease = manager.acquire(&info.id).unwrap();
    assert_eq!(
        manager
            .update(&info.id, "x".into(), ExecutionMode::Replica)
            .unwrap_err()
            .code,
        "BUSY"
    );
    drop(lease);
    // 名称与模式都未变化：不写入即可返回当前记录。
    let same = manager
        .update(&info.id, info.name.clone(), ExecutionMode::Replica)
        .unwrap();
    assert_eq!(same.name, info.name);
    assert_eq!(same.execution_mode, ExecutionMode::Replica);
}
