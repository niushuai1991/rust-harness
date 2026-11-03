mod commands;
mod desktop;
use tauri::Manager;

pub fn export_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let builder = commands::builder();
    builder.export(
        specta_typescript::Typescript::default(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/bindings.ts"),
    )?;
    Ok(())
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("harness_runtime=info")
        .try_init();
    let commands = commands::builder();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(commands.invoke_handler());
    // Public releases opt in only after configuring a real verification key and endpoint.
    #[cfg(feature = "signed-updates")]
    let app = app.plugin(tauri_plugin_updater::Builder::new().build());
    app.setup(|app| {
        let root = std::env::var_os("RUST_HARNESS_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or(
                dirs::data_local_dir()
                    .ok_or("找不到本地数据目录")?
                    .join("RustHarness"),
            );
        let runtime = tauri::async_runtime::block_on(harness_runtime::Runtime::open(root))?;
        let orchestration = tauri::async_runtime::block_on(async {
            harness_orchestration::install(runtime.clone())
        })?;
        app.manage(orchestration);
        app.manage(runtime);
        desktop::setup(app)?;
        Ok(())
    })
    .run(tauri::generate_context!())
    .expect("无法启动 Rust Harness");
}
