mod artifact_commands;
mod commands;
mod desktop;
mod plugin_commands;
pub mod uninstall;
mod web_panel;
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
        // 桌面默认权限模式（KV 持久化）→ 运行时默认档：覆盖新会话与定时任务；会话内切档即时生效。
        tauri::async_runtime::block_on(async {
            let stored = runtime
                .store
                .get(commands::DEFAULT_APPROVAL_MODE_KEY)
                .await
                .ok()
                .flatten()
                .and_then(|v| {
                    v.as_str()
                        .and_then(harness_contracts::ApprovalPolicy::parse)
                })
                .unwrap_or(harness_contracts::ApprovalPolicy::Ask);
            runtime.set_default_approval_policy(Some(stored));
        });
        let orchestration = tauri::async_runtime::block_on(async {
            harness_orchestration::install(runtime.clone())
        })?;
        app.manage(orchestration);
        let browser =
            tauri::async_runtime::block_on(async { harness_browser::install(runtime.clone()) })?;
        app.manage(browser);
        app.manage(runtime);
        desktop::setup(app)?;
        #[cfg(windows)]
        disable_webview2_browser_chrome(app.handle());
        Ok(())
    })
    .run(context)
    .expect("无法启动 Rust Harness");
}

/// 去掉 WebView2 自带的浏览器外壳行为：默认右键菜单（刷新、打印等）和
/// F5/Ctrl+R/Ctrl+P 等浏览器快捷键。tauri 未暴露对应开关，参照 wry
/// `webview2/mod.rs` 中 set_webview_settings 的做法直接调 COM 设置；
/// 不影响 Ctrl+C/V/X 等文本编辑快捷键。
#[cfg(windows)]
fn disable_webview2_browser_chrome(app: &tauri::AppHandle) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows::core::Interface;

    let Some(window) = app.get_webview_window("main") else {
        tracing::warn!("未找到主窗口，跳过关闭 WebView2 浏览器默认行为");
        return;
    };
    if let Err(error) = window.with_webview(move |webview| {
        let apply = || -> windows::core::Result<()> {
            unsafe {
                let core = webview.controller().CoreWebView2()?;
                let settings = core.Settings()?;
                settings.SetAreDefaultContextMenusEnabled(false)?;
                if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                    settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
                }
            }
            Ok(())
        };
        if let Err(error) = apply() {
            tracing::warn!("关闭 WebView2 默认右键菜单/浏览器快捷键失败：{error}");
        }
    }) {
        tracing::warn!("无法访问 WebView2 平台句柄：{error}");
    }
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

#[cfg(test)]
mod web_link_tests {
    #[test]
    fn accepts_only_absolute_http_and_https_urls() {
        for url in ["https://example.com/a", "http://localhost:8080/"] {
            assert!(super::web_panel::validated_url(url).is_ok());
        }
        assert!(super::web_panel::external_url("http://127.0.0.1:1420/").is_ok());
        for url in [
            "javascript:alert(1)",
            "file:///C:/secret",
            "//example.com",
            "https://",
            "http://tauri.localhost/",
            "http://127.0.0.1:1420/",
        ] {
            assert!(super::web_panel::validated_url(url).is_err());
        }
    }
}
