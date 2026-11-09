use harness_runtime::Runtime;
use std::path::Path;

#[tokio::test]
async fn import_chunks_are_scoped_persistent_integrity_checked_and_save_full_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let data = dir.path().join("host");
    let runtime = Runtime::open(data.clone()).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "attachments")
        .await
        .unwrap();
    let other = runtime
        .store
        .create_session(&workspace.id, "other")
        .await
        .unwrap();
    let input = dir.path().join("中文.txt");
    let text = "前".repeat(12000) + "结束";
    std::fs::write(&input, &text).unwrap();
    let meta = runtime.artifact_import(&session.id, &input).await.unwrap();
    assert_eq!(meta.name, "中文.txt");
    assert!(
        runtime
            .artifact_read(&other.id, &meta.hash, 0, 1024)
            .await
            .is_err()
    );
    let mut offset = 0;
    let mut received = String::new();
    loop {
        let chunk = runtime
            .artifact_read(&session.id, &meta.hash, offset, 32768)
            .await
            .unwrap();
        assert_eq!(chunk.encoding, "utf8");
        received.push_str(&chunk.content);
        match chunk.next_offset {
            Some(next) => {
                assert!(next > offset);
                offset = next
            }
            None => break,
        }
    }
    assert_eq!(received, text);
    assert_eq!(std::fs::read_to_string(&input).unwrap(), text);
    drop(runtime);
    let runtime = Runtime::open(data.clone()).await.unwrap();
    let out = dir.path().join("saved.txt");
    runtime
        .artifact_save(&session.id, &meta.hash, &out)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), text.as_bytes());
    std::fs::write(data.join("artifacts").join(&meta.hash), b"corrupt").unwrap();
    assert!(
        runtime
            .artifact_read(&session.id, &meta.hash, 0, 1024)
            .await
            .is_err()
    );
    assert!(
        runtime
            .artifact_save(&session.id, &meta.hash, Path::new("unused"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn binary_is_downloadable_and_chunks_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let w = runtime.workspaces.create(&source, None).unwrap();
    let s = runtime.store.create_session(&w.id, "binary").await.unwrap();
    let input = dir.path().join("binary.bin");
    std::fs::write(&input, [0, 255, 128, 1]).unwrap();
    let meta = runtime.artifact_import(&s.id, &input).await.unwrap();
    let chunk = runtime
        .artifact_read(&s.id, &meta.hash, 0, 2)
        .await
        .unwrap();
    assert_eq!(chunk.encoding, "base64");
    assert_eq!(chunk.content, "AP8=");
    assert_eq!(chunk.next_offset, Some(2));
    assert!(
        runtime
            .artifact_read(&s.id, &meta.hash, 0, 0)
            .await
            .is_err()
    );
    assert!(
        runtime
            .artifact_read(&s.id, &meta.hash, 0, 32769)
            .await
            .is_err()
    );
    assert!(
        runtime
            .artifact_read(&s.id, &meta.hash, 5, 2)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn imports_reject_oversize_and_artifacts_do_not_leak_between_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let w = runtime.workspaces.create(&source, None).unwrap();
    let s = runtime.store.create_session(&w.id, "one").await.unwrap();
    let second_source = dir.path().join("second");
    std::fs::create_dir(&second_source).unwrap();
    let other_w = runtime.workspaces.create(&second_source, None).unwrap();
    let other = runtime
        .store
        .create_session(&other_w.id, "other")
        .await
        .unwrap();
    let input = dir.path().join("oversize.bin");
    std::fs::File::create(&input)
        .unwrap()
        .set_len(20 * 1024 * 1024 + 1)
        .unwrap();
    assert!(runtime.artifact_import(&s.id, &input).await.is_err());
    assert!(runtime.store.events(&s.id, 0).await.unwrap().is_empty());
    std::fs::write(&input, "字符").unwrap();
    let metadata = runtime.artifact_import(&s.id, &input).await.unwrap();
    assert!(
        runtime
            .artifact_read(&other.id, &metadata.hash, 0, 32768)
            .await
            .is_err()
    );
    assert!(
        runtime
            .artifact_read(&s.id, &metadata.hash, 1, 5)
            .await
            .is_err()
    );
    assert!(
        runtime
            .artifact_read(&s.id, &metadata.hash, 0, 1)
            .await
            .is_err()
    );
}
