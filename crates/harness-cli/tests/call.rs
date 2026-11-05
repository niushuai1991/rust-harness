use std::process::Command;
#[test]
fn call_executes_dispatch_and_prints_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "call",
            "workspace.list",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
}
