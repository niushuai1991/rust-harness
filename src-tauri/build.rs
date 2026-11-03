fn main() {
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/STACK:16777216");
        // Integration tests link rfd's TaskDialogIndirect too; unlike the app bin they
        // do not receive Tauri's embedded Common Controls v6 resource automatically.
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}\\windows-app.manifest",
            std::env::var("CARGO_MANIFEST_DIR").unwrap()
        );
    }
}
