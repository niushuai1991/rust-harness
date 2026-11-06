mod artifact_commands;
mod commands;
mod desktop;
mod plugin_commands;
pub mod uninstall;
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
    let mut context = tauri::generate_context!();
    if let Some(arguments) = p1_guest_browser_args() {
        for window in &mut context.config_mut().app.windows {
            window.additional_browser_args = Some(arguments.clone());
        }
    }
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
        let browser =
            tauri::async_runtime::block_on(async { harness_browser::install(runtime.clone()) })?;
        app.manage(browser);
        app.manage(runtime);
        desktop::setup(app)?;
        Ok(())
    })
    .run(context)
    .expect("无法启动 Rust Harness");
}

fn p1_guest_browser_args() -> Option<String> {
    p1_guest_browser_args_from(
        std::env::var("P1_SANDBOX_GUEST").ok().as_deref(),
        std::env::var("USERNAME").ok().as_deref(),
        std::env::var("RUST_HARNESS_P1_CDP_PORT").ok().as_deref(),
    )
}

fn p1_guest_browser_args_from(
    marker: Option<&str>,
    username: Option<&str>,
    port: Option<&str>,
) -> Option<String> {
    let port = port?.parse::<u16>().ok().filter(|port| *port >= 1024)?;
    if marker != Some("1") || !username?.eq_ignore_ascii_case("WDAGUtilityAccount") {
        return None;
    }
    Some(format!(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn accepts_debug_port_only_for_the_disposable_p1_guest() {
        assert_eq!(
            super::p1_guest_browser_args_from(Some("1"), Some("WDAGUtilityAccount"), Some("9222")),
            Some("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=9222".into())
        );
        assert_eq!(
            super::p1_guest_browser_args_from(Some("1"), Some("DesktopUser"), Some("9222")),
            None
        );
        assert_eq!(
            super::p1_guest_browser_args_from(Some("1"), Some("WDAGUtilityAccount"), Some("80")),
            None
        );
    }
}
