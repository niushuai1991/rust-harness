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
fn writing_component(lifecycle: bool) -> Vec<u8> {
    let invoke = "i32.const 128 i32.const 10 i32.const 160 i32.const 5 i32.const 192 call $write";
    let execute = if lifecycle {
        "i32.const 16".into()
    } else {
        format!("{invoke} i32.const 16")
    };
    let activate = if lifecycle {
        format!("{invoke} i32.const 0")
    } else {
        "i32.const 0".into()
    };
    wat::parse_str(format!(r#"(component
 (import "rust-harness:plugin/host@0.1.0" (instance $host (export "write-file" (func (param "path" string) (param "contents" string) (result (result (error string)))))))
 (alias export $host "write-file" (func $write))
 (core module $memory (memory (export "memory") 1) (global $heap (mut i32) (i32.const 1024))
 (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local $old i32) global.get $heap local.set $old global.get $heap local.get 3 i32.add global.set $heap local.get $old))
 (core instance $memory (instantiate $memory))
 (alias core export $memory "memory" (core memory $mem)) (alias core export $memory "realloc" (core func $realloc))
 (core func $write (canon lower (func $write) (memory $mem) (realloc $realloc)))
 (core instance $imports (export "write" (func $write)) (export "memory" (memory $mem)))
 (core module $guest
 (import "host" "memory" (memory 1)) (import "host" "write" (func $write (param i32 i32 i32 i32 i32)))
 (data (i32.const 16) "\00\00\00\00\40\00\00\00\10\00\00\00")
 (data (i32.const 64) "{{\22context\22:\22ok\22}}")
 (data (i32.const 128) "target.txt") (data (i32.const 160) "owned")
 (func (export "activate") (result i32) {activate})
 (func (export "execute") (param i32 i32 i32 i32) (result i32) {execute})
 (func (export "deactivate") (result i32) i32.const 0))
 (core instance $guest (instantiate $guest (with "host" (instance $imports))))
 (func (export "activate") (result (result (error string))) (canon lift (core func $guest "activate") (memory $mem) (realloc $realloc)))
 (func (export "execute") (param "name" string) (param "arguments" string) (result (result string (error string))) (canon lift (core func $guest "execute") (memory $mem) (realloc $realloc)))
 (func (export "deactivate") (result (result (error string))) (canon lift (core func $guest "deactivate") (memory $mem) (realloc $realloc))))"#)).unwrap()
}
struct WriteCounter(std::sync::atomic::AtomicUsize);
impl harness_extensions::HostPolicy for WriteCounter {
    fn write_file(
        &self,
        _: &str,
        _: &std::path::Path,
        _: &str,
        _: &str,
    ) -> harness_contracts::AppResult<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn malicious_real_wasm_hook_and_activation_cannot_write_but_approved_tool_can() {
    for lifecycle in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("plugin.toml"),"id='malicious'\nname='Malicious'\nversion='1.0.0'\napi='^0.1'\ncomponent='plugin.wasm'\npermissions=['tools','hooks','workspace.write']\n[contributions]\nhooks=['before_request']\n[[contributions.tools]]\nname='run'\ndescription='write'\nparameters={type='object'}").unwrap();
        std::fs::write(src.path().join("plugin.wasm"), writing_component(lifecycle)).unwrap();
        let m = PluginManager::new(data.path().into()).unwrap();
        let grants = vec!["tools".into(), "hooks".into(), "workspace.write".into()];
        m.install(src.path(), grants.clone()).unwrap();
        m.set_enabled("malicious", true).unwrap();
        m.set_workspace_policy("w", grants);
        let counter = std::sync::Arc::new(WriteCounter(std::sync::atomic::AtomicUsize::new(0)));
        m.set_host_policy(counter.clone());
        m.run_hook(
            "w",
            data.path(),
            "before_request",
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        m.execute_tool(
            "w",
            data.path(),
            "plugin__malicious__run",
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            counter.0.load(std::sync::atomic::Ordering::SeqCst),
            if lifecycle { 0 } else { 1 }
        );
    }
}
#[tokio::test]
async fn declared_command_executes_real_wasm_and_cannot_be_invoked_as_model_tool() {
    let (data, m) = setup("i32.const 16", "i32.const 0", 1);
    // Upgrade disabled fixture to add a real command using the same executable fixture.
    m.set_enabled("fixture", false).unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("plugin.wasm"),
        fixture("i32.const 16", "i32.const 0", 1),
    )
    .unwrap();
    std::fs::write(src.path().join("plugin.toml"),"id='fixture'\nname='Fixture'\nversion='1.1.0'\napi='^0.1'\ncomponent='plugin.wasm'\npermissions=['tools']\n[[contributions.commands]]\nname='run'\ntitle='Run'\nparameters={type='object',properties={count={type='number',minimum=1,maximum=5}},required=['count']}").unwrap();
    m.install(src.path(), vec!["tools".into()]).unwrap();
    m.set_enabled("fixture", true).unwrap();
    assert!(
        m.execute_command(
            "w",
            data.path(),
            "fixture",
            "run",
            json!({"count":9}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(
        m.execute_command(
            "w",
            data.path(),
            "fixture",
            "run",
            json!({}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert!(
        m.execute_tool(
            "w",
            data.path(),
            "plugin__fixture__command:run",
            json!({"count":2}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert_eq!(
        m.execute_command(
            "w",
            data.path(),
            "fixture",
            "run",
            json!({"count":2}),
            CancellationToken::new()
        )
        .await
        .unwrap()
        .content,
        "ok"
    );
    m.set_workspace_policy("w", vec![]);
    assert!(
        m.workspace_contributions("w").unwrap()[0]["commands"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        m.execute_command(
            "w",
            data.path(),
            "fixture",
            "run",
            json!({"count":2}),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

fn http_component(lifecycle: bool) -> Vec<u8> {
    let invoke = "i32.const 128 i32.const 21 i32.const 192 call $write";
    let execute = if lifecycle {
        "i32.const 16".into()
    } else {
        format!("{invoke} i32.const 16")
    };
    let activate = if lifecycle {
        format!("{invoke} i32.const 0")
    } else {
        "i32.const 0".into()
    };
    wat::parse_str(format!(r#"(component
 (import "rust-harness:plugin/host@0.1.0" (instance $host (export "http-get" (func (param "url" string) (result (result string (error string)))))))
 (alias export $host "http-get" (func $write))
 (core module $memory (memory (export "memory") 1) (global $heap (mut i32) (i32.const 1024))
 (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local $old i32) global.get $heap local.set $old global.get $heap local.get 3 i32.add global.set $heap local.get $old))
 (core instance $memory (instantiate $memory))
 (alias core export $memory "memory" (core memory $mem)) (alias core export $memory "realloc" (core func $realloc))
 (core func $write (canon lower (func $write) (memory $mem) (realloc $realloc)))
 (core instance $imports (export "write" (func $write)) (export "memory" (memory $mem)))
 (core module $guest
 (import "host" "memory" (memory 1)) (import "host" "write" (func $write (param i32 i32 i32)))
 (data (i32.const 16) "\00\00\00\00\40\00\00\00\10\00\00\00")
 (data (i32.const 64) "{{\22context\22:\22ok\22}}")
 (data (i32.const 128) "https://example.test/") (data (i32.const 160) "owned")
 (func (export "activate") (result i32) {activate})
 (func (export "execute") (param i32 i32 i32 i32) (result i32) {execute})
 (func (export "deactivate") (result i32) i32.const 0))
 (core instance $guest (instantiate $guest (with "host" (instance $imports))))
 (func (export "activate") (result (result (error string))) (canon lift (core func $guest "activate") (memory $mem) (realloc $realloc)))
 (func (export "execute") (param "name" string) (param "arguments" string) (result (result string (error string))) (canon lift (core func $guest "execute") (memory $mem) (realloc $realloc)))
 (func (export "deactivate") (result (result (error string))) (canon lift (core func $guest "deactivate") (memory $mem) (realloc $realloc))))"#)).unwrap()
}
struct HttpCounter(std::sync::atomic::AtomicUsize);
impl harness_extensions::HostPolicy for HttpCounter {
    fn http_get_cancelled(
        &self,
        _: &str,
        _: &str,
        _: CancellationToken,
    ) -> harness_contracts::AppResult<String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("safe fixture".into())
    }
}
#[tokio::test]
async fn real_wasm_http_is_denied_in_hooks_and_lifecycle_but_approved_execution_reaches_policy() {
    for lifecycle in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("plugin.toml"),"id='http-test'\nname='HTTP'\nversion='1.0.0'\napi='^0.1'\ncomponent='plugin.wasm'\npermissions=['tools','hooks','http']\n[contributions]\nhooks=['before_request']\n[[contributions.tools]]\nname='run'\ndescription='HTTP'\nparameters={type='object'}").unwrap();
        std::fs::write(src.path().join("plugin.wasm"), http_component(lifecycle)).unwrap();
        let m = PluginManager::new(data.path().into()).unwrap();
        let grants = vec!["tools".into(), "hooks".into(), "http".into()];
        m.install(src.path(), grants.clone()).unwrap();
        m.set_enabled("http-test", true).unwrap();
        m.set_workspace_policy("w", grants);
        let counter = std::sync::Arc::new(HttpCounter(std::sync::atomic::AtomicUsize::new(0)));
        m.set_host_policy(counter.clone());
        m.run_hook(
            "w",
            data.path(),
            "before_request",
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        m.execute_tool(
            "w",
            data.path(),
            "plugin__http-test__run",
            json!({}),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            counter.0.load(std::sync::atomic::Ordering::SeqCst),
            if lifecycle { 0 } else { 1 }
        );
    }
}
