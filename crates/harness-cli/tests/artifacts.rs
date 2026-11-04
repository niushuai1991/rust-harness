use harness_cli::dispatch;
use harness_runtime::Runtime;
use serde_json::json;
#[tokio::test]
async fn headless_attachment_import_read_save_enforces_session_scope() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let w = runtime.workspaces.create(&source).unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "attachments")
        .await
        .unwrap();
    let other = runtime.store.create_session(&w.id, "other").await.unwrap();
    let input = dir.path().join("input.txt");
    std::fs::write(&input, "headless真实附件").unwrap();
    let metadata = dispatch(
        runtime.clone(),
        "artifact.import",
        json!({"sessionId":s.id,"path":input}),
    )
    .await
    .unwrap();
    let chunk = dispatch(
        runtime.clone(),
        "artifact.read",
        json!({"sessionId":s.id,"hash":metadata["hash"],"offset":0,"length":32768}),
    )
    .await
    .unwrap();
    assert_eq!(chunk["content"], "headless真实附件");
    assert!(
        dispatch(
            runtime.clone(),
            "artifact.read",
            json!({"sessionId":other.id,"hash":metadata["hash"],"offset":0,"length":32768})
        )
        .await
        .is_err()
    );
    let output = dir.path().join("download.txt");
    dispatch(
        runtime,
        "artifact.save",
        json!({"sessionId":s.id,"hash":metadata["hash"],"path":output}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(output).unwrap(),
        std::fs::read(input).unwrap()
    );
}
