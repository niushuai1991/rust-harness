use harness_runtime::Runtime;
use serde_json::json;
use std::time::Duration;
const COMPONENT: &str = r#"(component
(core module $m (memory (export "memory") 1) (data(i32.const 16) "\00\00\00\00\40\00\00\00\02\00\00\00") (data(i32.const 64) "ok")
(global $heap(mut i32)(i32.const 1024))(func(export "realloc")(param i32 i32 i32 i32)(result i32)(local $old i32)global.get $heap local.set $old global.get $heap local.get 3 i32.add global.set $heap local.get $old)
(func(export "activate")(result i32)i32.const 0)(func(export "execute")(param i32 i32 i32 i32)(result i32)i32.const 16)(func(export "deactivate")(result i32)i32.const 0))
(core instance $i(instantiate $m))
(alias core export $i "memory" (core memory $mem)) (alias core export $i "realloc" (core func $realloc))
(func(export "activate")(result(result(error string)))(canon lift(core func $i "activate")(memory $mem)(realloc $realloc)))
(func(export "execute")(param "name" string)(param "arguments" string)(result(result string(error string)))(canon lift(core func $i "execute")(memory $mem)(realloc $realloc)))
(func(export "deactivate")(result(result(error string)))(canon lift(core func $i "deactivate")(memory $mem)(realloc $realloc))))"#;
async fn setup() -> (tempfile::TempDir, std::sync::Arc<Runtime>, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let w = runtime
        .workspaces
        .create(&source, None, harness_contracts::ExecutionMode::Replica)
        .unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "plugins")
        .await
        .unwrap();
    let plugin = temp.path().join("plugin");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("plugin.wasm"), COMPONENT).unwrap();
    std::fs::write(plugin.join("plugin.toml"),"id='human'\nname='Human'\nversion='1.0.0'\napi='^0.1'\ncomponent='plugin.wasm'\npermissions=['tools','kv']\n[contributions.settings.count]\ntype='number'\ndefault=1\nminimum=1\nmaximum=5\n[[contributions.commands]]\nname='run'\ntitle='Run'").unwrap();
    runtime
        .plugins
        .install(&plugin, vec!["tools".into(), "kv".into()])
        .unwrap();
    runtime.plugins.set_enabled("human", true).unwrap();
    (temp, runtime, w.id, s.id)
}
async fn pending(runtime: &Runtime) -> harness_contracts::ApprovalRequest {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(r) = runtime.approvals.list().first() {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn first_session_settings_and_human_command_need_no_model_turn_and_settle_durably() {
    let (_data, r, w, s) = setup().await;
    assert_eq!(r.plugin_contributions(&w).unwrap()[0]["values"]["count"], 1);
    r.plugin_settings_save(&w, "human", json!({"count":4}))
        .await
        .unwrap();
    assert_eq!(r.plugin_contributions(&w).unwrap()[0]["values"]["count"], 4);
    let task = {
        let r = r.clone();
        let s = s.clone();
        tokio::spawn(async move { r.plugin_command(&s, "human", "run", json!({})).await })
    };
    let approval = pending(&r).await;
    assert_eq!(approval.tool, "plugin__human__command:run");
    r.approvals.resolve(&approval.id, true).unwrap();
    assert_eq!(task.await.unwrap().unwrap().content, "ok");
    let events = r.store.events(&s, 0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "plugin.command.result")
            .count(),
        1
    );
    assert!(!events.iter().any(|e| e.kind == "tool.result"));
    assert!(!r.is_active(&s));
    let task = {
        let r = r.clone();
        let s = s.clone();
        tokio::spawn(async move { r.plugin_command(&s, "human", "run", json!({})).await })
    };
    pending(&r).await;
    r.cancel(&s);
    assert!(task.await.unwrap().is_err());
    let events = r.store.events(&s, 0).await.unwrap();
    assert_eq!(events.last().unwrap().kind, "plugin.command.result");
    assert_eq!(events.last().unwrap().data["status"], "cancelled");
    assert!(r.approvals.list().is_empty());
    assert!(!r.is_active(&s));
}
#[tokio::test]
async fn command_session_scope_missing_plugin_and_unload_fail_closed() {
    let (_data, r, w, s) = setup().await;
    assert!(
        r.plugin_command("missing", "human", "run", json!({}))
            .await
            .is_err()
    );
    r.plugins.set_enabled("human", false).unwrap();
    assert!(
        r.plugin_command(&s, "human", "run", json!({}))
            .await
            .is_err()
    );
    assert!(r.plugin_contributions(&w).unwrap().is_empty());
    assert!(
        r.plugin_settings_save(&w, "human", json!({"count":2}))
            .await
            .is_err()
    );
    assert!(r.approvals.list().is_empty());
}
#[tokio::test]
async fn plugin_http_bridge_cancels_native_request_and_bounds_declared_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for cancel_request in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let (sent, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            let _ = socket.read(&mut buf).await;
            let headers = if cancel_request {
                b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n".as_slice()
            };
            socket.write_all(headers).await.unwrap();
            let _ = sent.send(());
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let cancel = tokio_util::sync::CancellationToken::new();
        let child = cancel.clone();
        let task =
            tokio::task::spawn_blocking(move || harness_runtime::plugin_ui::http_get(&url, child));
        received.await.unwrap();
        if cancel_request {
            cancel.cancel()
        }
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(message.contains(if cancel_request {
            "cancelled"
        } else {
            "body limit"
        }));
        server.abort();
    }
    let result = tokio::task::spawn_blocking(|| {
        harness_runtime::plugin_ui::http_get(
            "https://user:secret@example.test",
            tokio_util::sync::CancellationToken::new(),
        )
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!result.unwrap_err().to_string().contains("secret"));
}
#[tokio::test]
async fn disabling_plugin_cancels_waiting_human_command_approval() {
    let (_data, r, _w, s) = setup().await;
    let task = {
        let r = r.clone();
        let s = s.clone();
        tokio::spawn(async move { r.plugin_command(&s, "human", "run", json!({})).await })
    };
    pending(&r).await;
    r.plugins.set_enabled("human", false).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(r.approvals.list().is_empty());
    assert!(!r.is_active(&s));
    assert_eq!(
        r.store.events(&s, 0).await.unwrap().last().unwrap().data["status"],
        "cancelled"
    );
}
