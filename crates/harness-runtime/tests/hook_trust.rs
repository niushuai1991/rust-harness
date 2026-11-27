use harness_runtime::{Runtime, hook_trust::HookTrustService};
use serde_json::json;

#[tokio::test]
async fn project_hook_trust_is_exact_and_script_changes_require_review() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join(".rh")).unwrap();
    let script = root.join(".rh/check.ps1");
    std::fs::write(&script, "exit 0").unwrap();
    let settings = json!({"hooks":{"PreToolUse":[{"matcher":"write","hooks":[{"type":"command","command":"powershell.exe","args":["-NoProfile","-File",".rh/check.ps1"]}]}]}});
    std::fs::write(
        root.join(".rh/.settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let service = HookTrustService::new(runtime.store.clone());
    let before = service.review(&root).await.unwrap();
    assert_eq!(before.len(), 1);
    assert!(!before[0].trusted);
    assert_eq!(
        service.require_trust(&root).await.unwrap_err().code,
        "HOOK_TRUST_REQUIRED"
    );
    let first_digest = before[0].digest.clone();
    service.trust(&root, &first_digest).await.unwrap();
    assert!(service.review(&root).await.unwrap()[0].trusted);
    service.require_trust(&root).await.unwrap();
    std::fs::write(&script, "exit 2").unwrap();
    let changed = service.review(&root).await.unwrap();
    assert!(!changed[0].trusted);
    assert_eq!(
        service.require_trust(&root).await.unwrap_err().code,
        "HOOK_TRUST_REQUIRED"
    );
    assert_ne!(changed[0].digest, first_digest);
    assert!(service.trust(&root, &first_digest).await.is_err());
    service.trust(&root, &changed[0].digest).await.unwrap();
    service.revoke(&root, &changed[0].digest).await.unwrap();
    assert!(!service.review(&root).await.unwrap()[0].trusted);
}

#[tokio::test]
async fn local_settings_trust_persists_but_does_not_transfer_to_another_project() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for root in [&first, &second] {
        std::fs::create_dir_all(root.join(".rh")).unwrap();
        std::fs::write(
            root.join(".rh/.settings-local.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"exit 0"}]}]}}"#,
        )
        .unwrap();
    }
    let runtime = Runtime::open(host.clone()).await.unwrap();
    let service = HookTrustService::new(runtime.store.clone());
    let digest = service.review(&first).await.unwrap()[0].digest.clone();
    assert_ne!(digest, service.review(&second).await.unwrap()[0].digest);
    service.trust(&first, &digest).await.unwrap();
    assert!(!service.review(&second).await.unwrap()[0].trusted);
    drop(service);
    drop(runtime);
    let runtime = Runtime::open(host).await.unwrap();
    let service = HookTrustService::new(runtime.store.clone());
    assert!(service.review(&first).await.unwrap()[0].trusted);
    assert!(!service.review(&second).await.unwrap()[0].trusted);
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_or_linked_project_hook_files_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    symlink(root.join("missing"), root.join(".rh")).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let service = HookTrustService::new(runtime.store.clone());
    assert!(service.review(&root).await.is_err());
}
