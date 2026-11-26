use harness_cli::dispatch;
use harness_runtime::Runtime;
use serde_json::json;

#[tokio::test]
async fn mcp_config_rpc_persists_and_resolves_project_override() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let global = json!({"id":"test","transport":{"type":"stdio","command":"global"}});
    let project = json!({"id":"test","transport":{"type":"stdio","command":"project"}});
    dispatch(
        runtime.clone(),
        "mcp.config.save",
        json!({"scope":{"kind":"global"},"config":global}),
    )
    .await
    .unwrap();
    dispatch(
        runtime.clone(),
        "mcp.config.save",
        json!({"scope":{"kind":"project","workspaceId":"w"},"config":project}),
    )
    .await
    .unwrap();
    let result = dispatch(
        runtime.clone(),
        "mcp.config.list",
        json!({"workspaceId":"w"}),
    )
    .await
    .unwrap();
    assert_eq!(result[0]["config"]["transport"]["command"], "project");
    assert_eq!(result[0]["trusted"], false);
    dispatch(
        runtime.clone(),
        "mcp.config.trust",
        json!({"scope":{"kind":"project","workspaceId":"w"},"config":project}),
    )
    .await
    .unwrap();
    assert_eq!(
        dispatch(runtime, "mcp.config.list", json!({"workspaceId":"w"}))
            .await
            .unwrap()[0]["trusted"],
        true
    );
}
#[tokio::test]
async fn mcp_config_connect_checks_that_saved_server_exists() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let session = runtime.store.create_session("", "session").await.unwrap();
    let error = dispatch(
        runtime,
        "mcp.config.connect",
        json!({"sessionId":session.id,"id":"missing"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "MCP_CONFIG");
}
