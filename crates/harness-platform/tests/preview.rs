#[test]
fn preview_is_bounded_and_rejects_escape() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.txt"), vec![b'a'; 100000]).unwrap();
    assert_eq!(
        harness_platform::read_file_bounded(root.path(), "file.txt", 1024)
            .unwrap()
            .len(),
        1024
    );
    for path in ["../outside", "C:/secret", "file.txt:stream"] {
        assert!(harness_platform::read_file_bounded(root.path(), path, 1024).is_err());
    }
}
