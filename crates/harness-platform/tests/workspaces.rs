use harness_platform::*;
use std::{fs, path::PathBuf};
fn fixture() -> (tempfile::TempDir, PathBuf, WorkspaceManager) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests");
    fs::create_dir_all(&base).unwrap();
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
    let info = manager.create(&source).unwrap();
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
    let info = manager.create(&source).unwrap();
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
    let info = manager.create(&source).unwrap();
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
    assert!(nested.create(&source).is_err());
    let _ = manager;
}

#[test]
fn ignored_caches_are_not_copied_and_hardlinks_are_rejected() {
    let (_temp, source, manager) = fixture();
    fs::write(source.join(".gitignore"), "cache/\n").unwrap();
    fs::create_dir(source.join("cache")).unwrap();
    fs::write(source.join("cache/large"), "ignored").unwrap();
    let info = manager.create(&source).unwrap();
    assert!(
        !std::path::Path::new(&info.replica_path)
            .join("cache")
            .exists()
    );
    fs::hard_link(source.join("a.txt"), source.join("linked.txt")).unwrap();
    assert_eq!(manager.create(&source).unwrap_err().code, "UNSAFE_LINK");
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
    let info = manager.create(&source).unwrap();
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
    let info = manager.create(&source).unwrap();
    assert_eq!(
        read_file(std::path::Path::new(&info.replica_path), "a.txt").unwrap(),
        "initial"
    );
}

#[test]
fn new_ignore_rule_does_not_turn_baseline_file_into_a_deletion() {
    let (_temp, source, manager) = fixture();
    let info = manager.create(&source).unwrap();
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
    let info = manager.create(&source).unwrap();
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
    let info = manager.create(&source).unwrap();
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
