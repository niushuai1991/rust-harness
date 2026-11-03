use harness_extensions::PluginManager;
use serde_json::json;
use tokio_util::sync::CancellationToken;
fn fixture(execute: &str, activate: &str, memory: u32) -> Vec<u8> {
    fixture_output(execute, activate, memory, "ok")
}
fn fixture_output(execute: &str, activate: &str, memory: u32, response: &str) -> Vec<u8> {
    let encoded = response
        .as_bytes()
        .iter()
        .map(|b| format!("\\{b:02x}"))
        .collect::<String>();
    let length = (response.len() as u32)
        .to_le_bytes()
        .iter()
        .map(|b| format!("\\{b:02x}"))
        .collect::<String>();
    wat::parse_str(format!(r#"(component
 (core module $m
  (memory (export "memory") {memory})
  (data (i32.const 16) "\00\00\00\00\40\00\00\00{length}")
  (data (i32.const 64) "{encoded}")
  (global $heap (mut i32) (i32.const 1024))
  (func (export "realloc") (param i32 i32 i32 i32) (result i32)
   (local $old i32) global.get $heap local.set $old global.get $heap local.get 3 i32.add global.set $heap local.get $old)
  (func (export "activate") (result i32) {activate})
  (func (export "execute") (param i32 i32 i32 i32) (result i32) {execute})
  (func (export "deactivate") (result i32) i32.const 0))
 (core instance $i (instantiate $m))
 (alias core export $i "memory" (core memory $mem))
 (alias core export $i "realloc" (core func $realloc))
 (alias core export $i "activate" (core func $activate))
 (alias core export $i "execute" (core func $execute))
 (alias core export $i "deactivate" (core func $deactivate))
 (func (export "activate") (result (result (error string))) (canon lift (core func $activate) (memory $mem) (realloc $realloc)))
 (func (export "execute") (param "name" string) (param "arguments" string) (result (result string (error string))) (canon lift (core func $execute) (memory $mem) (realloc $realloc)))
 (func (export "deactivate") (result (result (error string))) (canon lift (core func $deactivate) (memory $mem) (realloc $realloc))))"#)).unwrap()
}
fn setup(execute: &str, activate: &str, memory: u32) -> (tempfile::TempDir, PluginManager) {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("plugin.toml"),
        r#"id='fixture'
name='Fixture'
version='1.0.0'
api='^0.1'
component='plugin.wasm'
permissions=['tools']
[[contributions.tools]]
name='run'
description='real component'
parameters={type='object'}
"#,
    )
    .unwrap();
    std::fs::write(
        src.path().join("plugin.wasm"),
        fixture(execute, activate, memory),
    )
    .unwrap();
    let manager = PluginManager::new(data.path().into()).unwrap();
    manager.install(src.path(), vec!["tools".into()]).unwrap();
    manager.set_enabled("fixture", true).unwrap();
    manager.set_workspace_policy("w", vec!["tools".into()]);
    (data, manager)
}
#[tokio::test]
async fn real_component_executes_with_explicit_workspace_grant() {
    let (data, m) = setup("i32.const 16", "i32.const 0", 1);
    assert_eq!(m.tool_definitions("w").unwrap().len(), 1);
    assert!(m.tool_definitions("untrusted").unwrap().is_empty());
    assert!(
        m.execute_tool(
            "untrusted",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert_eq!(
        m.execute_tool(
            "w",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .unwrap()
        .content,
        "ok"
    );
    m.set_enabled("fixture", false).unwrap();
    assert!(
        m.execute_tool(
            "w",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn infinite_component_is_cancelled_and_disable_blocks_new_calls() {
    let (data, m) = setup("(loop $l br $l) unreachable", "i32.const 0", 1);
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        t.cancel();
    });
    let start = std::time::Instant::now();
    assert!(
        m.execute_tool("w", data.path(), "plugin__fixture__run", json!({}), token)
            .await
            .is_err()
    );
    assert!(start.elapsed().as_secs() < 5);
}
#[tokio::test]
async fn oversized_linear_memory_is_rejected() {
    let (data, m) = setup("i32.const 16", "i32.const 0", 4097);
    assert!(
        m.execute_tool(
            "w",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn activation_failure_never_returns_contributions_from_guest() {
    let (data, m) = setup("i32.const 16", "unreachable", 1);
    assert!(
        m.execute_tool(
            "w",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!data.path().join("kv/fixture/77.json").exists());
}
#[tokio::test]
async fn disabling_cancels_inflight_component() {
    let (data, m) = setup("(loop $l br $l) unreachable", "i32.const 0", 1);
    let m = std::sync::Arc::new(m);
    let runner = m.clone();
    let root = data.path().to_owned();
    let call = tokio::spawn(async move {
        runner
            .execute_tool(
                "w",
                &root,
                "plugin__fixture__run",
                json!({}),
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    m.set_enabled("fixture", false).unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), call)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
fn hook_setup(response: &str) -> (tempfile::TempDir, PluginManager) {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("plugin.toml"),"id='hooks'\nname='Hooks'\nversion='1.0.0'\napi='^0.1'\ncomponent='plugin.wasm'\npermissions=['tools','hooks']\n[contributions]\nhooks=['before_request']\n").unwrap();
    std::fs::write(
        src.path().join("plugin.wasm"),
        fixture_output("i32.const 16", "i32.const 0", 1, response),
    )
    .unwrap();
    let m = PluginManager::new(data.path().into()).unwrap();
    m.install(src.path(), vec!["tools".into(), "hooks".into()])
        .unwrap();
    m.set_enabled("hooks", true).unwrap();
    m.set_workspace_policy("w", vec!["tools".into(), "hooks".into()]);
    (data, m)
}
#[tokio::test]
async fn declared_hook_executes_real_component_and_requires_separate_permission() {
    let (data, m) = hook_setup(r#"{"context":"from actual wasm"}"#);
    let results = m
        .run_hook(
            "w",
            data.path(),
            "before_request",
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(results, vec![json!({"context":"from actual wasm"})]);
    assert!(
        m.run_hook(
            "w",
            data.path(),
            "before_tool",
            json!({}),
            CancellationToken::new()
        )
        .await
        .unwrap()
        .is_empty()
    );
    m.set_workspace_policy("w", vec!["tools".into()]);
    assert!(
        m.run_hook(
            "w",
            data.path(),
            "before_request",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn invalid_hook_result_is_rejected_before_kv_commit() {
    let (data, m) = hook_setup(r#"{"bad":"output"}"#);
    assert!(
        m.run_hook(
            "w",
            data.path(),
            "before_request",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(!data.path().join("kv/hooks/77.json").exists());
}
#[tokio::test]
async fn infinite_activation_is_bounded_without_external_cancel() {
    let (data, m) = setup("i32.const 16", "(loop $l br $l) unreachable", 1);
    let start = std::time::Instant::now();
    let result = m
        .execute_tool(
            "w",
            data.path(),
            "plugin__fixture__run",
            json!({}),
            CancellationToken::new(),
        )
        .await;
    assert!(result.is_err());
    assert!(start.elapsed().as_secs() < 8);
    assert!(!data.path().join("kv/fixture/77.json").exists());
}
