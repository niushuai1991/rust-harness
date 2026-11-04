use harness_extensions::PluginManager;
use serde_json::json;
fn setup(schema: &str) -> (tempfile::TempDir, PluginManager) {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("plugin.toml"),
        format!(
            "id='forms'\nname='Forms'\nversion='1.0.0'\napi='^0.1'\npermissions=['kv']\n{schema}"
        ),
    )
    .unwrap();
    let m = PluginManager::new(data.path().into()).unwrap();
    m.install(src.path(), vec!["kv".into()]).unwrap();
    m.set_enabled("forms", true).unwrap();
    m.set_workspace_policy("w", vec!["kv".into()]);
    (data, m)
}
#[tokio::test]
async fn settings_validate_persist_scope_and_remove_on_disable() {
    let (data, m) =
        setup("[contributions.settings.limit]\ntype='number'\ndefault=2\nminimum=1\nmaximum=5");
    assert_eq!(
        m.workspace_contributions("w").unwrap()[0]["values"]["limit"],
        2
    );
    assert!(
        m.save_settings("w", "forms", json!({"limit":9}))
            .await
            .is_err()
    );
    m.save_settings("w", "forms", json!({"limit":4}))
        .await
        .unwrap();
    assert_eq!(
        m.workspace_contributions("w").unwrap()[0]["values"]["limit"],
        4
    );
    assert!(
        m.save_settings("other", "forms", json!({"limit":3}))
            .await
            .is_err()
    );
    drop(m);
    let m = PluginManager::new(data.path().into()).unwrap();
    m.set_workspace_policy("w", vec!["kv".into()]);
    assert_eq!(
        m.workspace_contributions("w").unwrap()[0]["values"]["limit"],
        4
    );
    m.set_enabled("forms", false).unwrap();
    assert!(m.workspace_contributions("w").unwrap().is_empty());
    assert!(
        m.save_settings("w", "forms", json!({"limit":3}))
            .await
            .is_err()
    );
}
#[test]
fn unsafe_themes_and_invalid_schema_are_rejected() {
    for declaration in [
        "[contributions.themes.bad]\n'--accent'='url(https://evil.test)'",
        "[contributions.settings.bad]\ntype='object'",
        "[contributions.themes.bad]\nposition='fixed'",
    ] {
        let outcome = std::panic::catch_unwind(|| setup(declaration));
        assert!(outcome.is_err());
    }
}
