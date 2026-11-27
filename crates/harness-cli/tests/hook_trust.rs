use harness_cli::dispatch;
use harness_contracts::ExecutionMode;
use harness_runtime::Runtime;
use serde_json::json;

#[tokio::test]
async fn project_hooks_can_be_reviewed_trusted_and_revoked_over_rpc() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(project.join(".rh")).unwrap();
    std::fs::write(
        project.join(".rh/.settings.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo done"}]}]}}"#,
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&project, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session_with_mode(&workspace.id, "hooks", Some(ExecutionMode::Direct))
        .await
        .unwrap();
    let params = json!({"sessionId":session.id});
    let review = dispatch(runtime.clone(), "hooks.project.review", params.clone())
        .await
        .unwrap();
    assert_eq!(review[0]["trusted"], false);
    let digest = review[0]["digest"].as_str().unwrap();
    let action = json!({"sessionId":session.id,"digest":digest});
    dispatch(runtime.clone(), "hooks.project.trust", action.clone())
        .await
        .unwrap();
    assert_eq!(
        dispatch(runtime.clone(), "hooks.project.review", params.clone())
            .await
            .unwrap()[0]["trusted"],
        true
    );
    dispatch(runtime.clone(), "hooks.project.revoke", action)
        .await
        .unwrap();
    assert_eq!(
        dispatch(runtime, "hooks.project.review", params)
            .await
            .unwrap()[0]["trusted"],
        false
    );
}
