//! 单元测试：URL 白名单、可执行文件候选与命令契约。
use harness_browser::command::{BrowserCommand, is_allowed_url};
use harness_browser::launcher::{candidate_roots, find_executable_with};
use std::path::PathBuf;

#[test]
fn url_allowlist_only_accepts_http_https_and_about_blank() {
    assert!(is_allowed_url("http://localhost:3000/"));
    assert!(is_allowed_url("https://example.com/x?y=1"));
    assert!(is_allowed_url("HTTPS://EXAMPLE.COM"));
    assert!(is_allowed_url("about:blank"));
    assert!(!is_allowed_url("file:///C:/Windows/system.ini"));
    assert!(!is_allowed_url("data:text/html,<b>hi</b>"));
    assert!(!is_allowed_url("javascript:alert(1)"));
    assert!(!is_allowed_url("ftp://example.com"));
    assert!(!is_allowed_url(""));
    assert!(!is_allowed_url("example.com"));
    assert!(!is_allowed_url("http://"));
    assert!(!is_allowed_url("ws://127.0.0.1:9222"));
}

#[test]
fn browser_candidates_prefer_edge_then_chrome_across_standard_dirs() {
    let env = |k: &str| -> Option<String> {
        match k {
            "ProgramFiles" => Some(r"C:\pf".into()),
            "ProgramFiles(x86)" => Some(r"C:\pf86".into()),
            "LocalAppData" => Some(r"C:\lad".into()),
            _ => None,
        }
    };
    let roots = candidate_roots(env);
    assert_eq!(
        roots[0],
        PathBuf::from(r"C:\pf\Microsoft\Edge\Application\msedge.exe")
    );
    assert!(roots.contains(&PathBuf::from(
        r"C:\pf86\Microsoft\Edge\Application\msedge.exe"
    )));
    assert!(roots.contains(&PathBuf::from(
        r"C:\lad\Microsoft\Edge\Application\msedge.exe"
    )));
    let edge = roots
        .iter()
        .position(|p| p.ends_with("msedge.exe"))
        .unwrap();
    let chrome = roots
        .iter()
        .position(|p| p.ends_with(r"Google\Chrome\Application\chrome.exe"))
        .unwrap();
    assert!(edge < chrome);
}

#[test]
fn explicit_executable_wins_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("my-browser.exe");
    std::fs::write(&exe, b"mz").unwrap();
    let found = find_executable_with(Some(exe.to_str().unwrap().to_string()), |_| None).unwrap();
    assert_eq!(found, exe);
}

#[test]
fn environment_override_is_used_before_system_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("env-browser.exe");
    std::fs::write(&exe, b"mz").unwrap();
    let found = find_executable_with(None, |k| {
        (k == "RUST_HARNESS_BROWSER_EXECUTABLE").then(|| exe.to_str().unwrap().to_string())
    })
    .unwrap();
    assert_eq!(found, exe);
}

#[test]
fn missing_everywhere_reports_actionable_error() {
    let err = find_executable_with(None, |_| None).unwrap_err();
    assert_eq!(err.code, "BROWSER_NOT_FOUND");
    assert!(err.message.contains("--browser-executable") || err.message.contains("浏览器"));
}

#[test]
fn version_subdir_executable_is_preferred_over_stub() {
    use harness_browser::launcher::resolve_real_executable;
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("msedge.exe");
    std::fs::write(&stub, b"mz").unwrap();
    let old = dir.path().join("155.0.1.1");
    let newest = dir.path().join("156.0.4314.58");
    let noise = dir.path().join("PlatformExperiencesHelper");
    std::fs::create_dir_all(old.join("sub")).unwrap();
    std::fs::create_dir_all(&newest).unwrap();
    std::fs::create_dir_all(&noise).unwrap();
    std::fs::write(newest.join("msedge.exe"), b"mz").unwrap();
    std::fs::write(old.join("sub").join("msedge.exe"), b"mz").unwrap();
    std::fs::write(noise.join("msedge.exe"), b"mz").unwrap();
    let resolved = resolve_real_executable(&stub);
    assert_eq!(resolved, newest.join("msedge.exe"));
}

#[test]
fn stub_without_version_dirs_stays_unchanged() {
    use harness_browser::launcher::resolve_real_executable;
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("chrome.exe");
    std::fs::write(&stub, b"mz").unwrap();
    assert_eq!(resolve_real_executable(&stub), stub);
}

#[test]
fn commands_serialize_with_action_tag() {
    let click: serde_json::Value =
        serde_json::to_value(BrowserCommand::Click { r#ref: "e1".into() }).unwrap();
    assert_eq!(click, serde_json::json!({"action":"click","ref":"e1"}));

    let navigate: BrowserCommand = serde_json::from_value(serde_json::json!({
        "action": "navigate",
        "url": "http://localhost:8000/"
    }))
    .unwrap();
    assert_eq!(
        navigate,
        BrowserCommand::Navigate {
            url: "http://localhost:8000/".into()
        }
    );

    let submit: serde_json::Value = serde_json::to_value(BrowserCommand::Type {
        r#ref: "e2".into(),
        text: "hello".into(),
        submit: true,
    })
    .unwrap();
    assert_eq!(
        submit,
        serde_json::json!({"action":"type","ref":"e2","text":"hello","submit":true})
    );
}
