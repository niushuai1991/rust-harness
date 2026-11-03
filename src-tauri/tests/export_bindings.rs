#[test]
fn export_frontend_bindings() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| rust_harness_desktop::export_bindings().unwrap())
        .unwrap()
        .join()
        .unwrap();
}
