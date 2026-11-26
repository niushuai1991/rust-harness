use harness_runtime::mcp_config::{McpConfigService, McpScope, McpServerConfig};
use harness_storage::Store;
use serde_json::json;

fn server(id: &str, command: &str) -> McpServerConfig {
    serde_json::from_value(json!({
        "id": id, "enabled": true,
        "transport": {"type":"stdio","command":command,"args":[]}
    }))
    .unwrap()
}

#[tokio::test]
async fn configuration_persists_and_project_overrides_global() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("store.db");
    let store = Store::open(&db).await.unwrap();
    let service = McpConfigService::new(store);
    service
        .save(McpScope::Global, server("sample", "global"))
        .await
        .unwrap();
    service
        .save(McpScope::Project("a".into()), server("sample", "project"))
        .await
        .unwrap();
    drop(service);
    let service = McpConfigService::new(Store::open(&db).await.unwrap());
    let project = service.effective(Some("a")).await.unwrap();
    assert_eq!(project.len(), 1);
    assert_eq!(
        serde_json::to_value(&project[0].config).unwrap()["transport"]["command"],
        "project"
    );
    assert!(!project[0].trusted);
    let global = service.effective(None).await.unwrap();
    assert_eq!(
        serde_json::to_value(&global[0].config).unwrap()["transport"]["command"],
        "global"
    );
    assert!(global[0].trusted);
}

#[tokio::test]
async fn trust_is_revoked_when_project_config_changes_and_invalid_save_preserves_old() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("store.db")).await.unwrap();
    let service = McpConfigService::new(store);
    let scope = McpScope::Project("a".into());
    service
        .save(scope.clone(), server("sample", "one"))
        .await
        .unwrap();
    service
        .trust_exact(scope.clone(), &server("sample", "one"))
        .await
        .unwrap();
    assert!(service.effective(Some("a")).await.unwrap()[0].trusted);
    service
        .save(scope.clone(), server("sample", "two"))
        .await
        .unwrap();
    assert!(!service.effective(Some("a")).await.unwrap()[0].trusted);
    assert!(service.save(scope, server("sample", "")).await.is_err());
    let current = service.effective(Some("a")).await.unwrap();
    assert_eq!(
        serde_json::to_value(&current[0].config).unwrap()["transport"]["command"],
        "two"
    );
}
#[tokio::test]
async fn trust_rejects_stale_configuration() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("store.db")).await.unwrap();
    let service = McpConfigService::new(store);
    let scope = McpScope::Project("a".into());
    let old = server("sample", "old");
    service.save(scope.clone(), old.clone()).await.unwrap();
    service
        .save(scope.clone(), server("sample", "new"))
        .await
        .unwrap();
    assert!(service.trust_exact(scope, &old).await.is_err());
    assert!(!service.effective(Some("a")).await.unwrap()[0].trusted);
}
#[tokio::test]
async fn reject_plaintext_secrets_and_invalid_addresses_without_overwriting() {
    let plaintext = json!({
        "id":"remote", "transport": {
            "type":"streamableHttp", "url":"https://example.invalid/mcp",
            "headers":{"Authorization":"secret"}
        }
    });
    assert!(serde_json::from_value::<McpServerConfig>(plaintext).is_err());
    let temp = tempfile::tempdir().unwrap();
    let service = McpConfigService::new(Store::open(&temp.path().join("host.db")).await.unwrap());
    let invalid: McpServerConfig = serde_json::from_value(json!({
        "id":"remote", "transport":{"type":"streamableHttp","url":"https://u:secret@example.invalid/mcp"}
    })).unwrap();
    assert!(service.save(McpScope::Global, invalid).await.is_err());
    assert!(service.effective(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn removing_project_override_reveals_global_entry() {
    let temp = tempfile::tempdir().unwrap();
    let service = McpConfigService::new(Store::open(&temp.path().join("host.db")).await.unwrap());
    let project = McpScope::Project("a".into());
    service
        .save(McpScope::Global, server("sample", "global"))
        .await
        .unwrap();
    service
        .save(project.clone(), server("sample", "project"))
        .await
        .unwrap();
    service.remove(project, "sample").await.unwrap();
    let result = service.effective(Some("a")).await.unwrap();
    assert_eq!(
        serde_json::to_value(&result[0].config).unwrap()["transport"]["command"],
        "global"
    );
}
#[tokio::test]
async fn runtime_reopens_saved_configuration() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("host");
    let runtime = harness_runtime::Runtime::open(path.clone()).await.unwrap();
    runtime
        .mcp_config
        .save(McpScope::Global, server("sample", "global"))
        .await
        .unwrap();
    drop(runtime);
    let runtime = harness_runtime::Runtime::open(path).await.unwrap();
    assert_eq!(
        runtime.mcp_config.effective(None).await.unwrap()[0]
            .config
            .id,
        "sample"
    );
}
#[tokio::test]
async fn invalid_large_timeouts_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let service = McpConfigService::new(Store::open(&temp.path().join("host.db")).await.unwrap());
    let mut config = server("timeout", "program");
    config.tool_timeout_secs = u64::MAX;
    assert!(service.save(McpScope::Global, config).await.is_err());
}
#[tokio::test]
async fn invalid_http_header_does_not_replace_saved_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let service = McpConfigService::new(Store::open(&temp.path().join("host.db")).await.unwrap());
    let valid:McpServerConfig=serde_json::from_value(json!({"id":"remote","transport":{"type":"streamableHttp","url":"https://example.invalid/mcp"}})).unwrap();
    service.save(McpScope::Global, valid.clone()).await.unwrap();
    let invalid:McpServerConfig=serde_json::from_value(json!({"id":"remote","transport":{"type":"streamableHttp","url":"https://example.invalid/mcp","headers":{"bad name":{"source":"environment","name":"TOKEN"}}}})).unwrap();
    assert!(service.save(McpScope::Global, invalid).await.is_err());
    assert_eq!(service.effective(None).await.unwrap()[0].config, valid);
}
#[tokio::test]
async fn unsafe_working_directory_is_rejected_before_saving() {
    let temp = tempfile::tempdir().unwrap();
    let service = McpConfigService::new(Store::open(&temp.path().join("host.db")).await.unwrap());
    let bad: McpServerConfig = serde_json::from_value(
        json!({"id":"local","transport":{"type":"stdio","command":"server","cwd":"../escape"}}),
    )
    .unwrap();
    assert!(service.save(McpScope::Global, bad).await.is_err());
    assert!(service.effective(None).await.unwrap().is_empty());
}
