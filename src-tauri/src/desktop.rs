//! Desktop lifecycle. Closing the window keeps sessions alive in the tray.
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
};

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "打开 Rust Harness", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Rust Harness", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("Rust Harness · 本地开发搭档")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => {
                let active = app
                    .try_state::<std::sync::Arc<harness_runtime::Runtime>>()
                    .is_some_and(|s| s.active_count() > 0)
                    || app
                        .try_state::<harness_orchestration::Installed>()
                        .is_some_and(|s| s.has_activity());
                if active {
                    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
                    let handle = app.clone();
                    app.dialog()
                        .message("正在运行的任务将会中断。确定退出 Rust Harness？")
                        .title("退出 Rust Harness")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "退出".into(),
                            "取消".into(),
                        ))
                        .show(move |confirmed| {
                            if confirmed {
                                if let Some(runtime) =
                                    handle.try_state::<std::sync::Arc<harness_runtime::Runtime>>()
                                {
                                    runtime.cancel_all();
                                }
                                handle.exit(0);
                            }
                        });
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    if let Some(window) = app.get_webview_window("main") {
        let handle = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if handle.hide().is_ok() {
                    api.prevent_close();
                }
            }
        });
    }
    Ok(())
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
