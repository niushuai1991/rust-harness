use tauri::{
    LogicalPosition, LogicalSize, Manager, WebviewUrl,
    webview::{NewWindowResponse, WebviewBuilder},
};
use tauri_plugin_opener::OpenerExt;

const LABEL: &str = "session-link-web";

pub(crate) fn external_url(raw: &str) -> Result<tauri::Url, String> {
    let url = tauri::Url::parse(raw).map_err(|_| "无效的网址".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("仅支持 HTTP 和 HTTPS 网址".into());
    }
    Ok(url)
}

pub(crate) fn validated_url(raw: &str) -> Result<tauri::Url, String> {
    let url = external_url(raw)?;
    let host = url.host_str().unwrap_or_default();
    if host.ends_with(".localhost")
        || (host == "127.0.0.1" && url.port_or_known_default() == Some(1420))
    {
        return Err("不能在网页面板中打开应用内部地址".into());
    }
    Ok(url)
}

#[tauri::command]
#[specta::specta]
pub async fn open_external_link(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let url = external_url(&url)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn web_panel_open(
    app: tauri::AppHandle,
    url: String,
    generation: String,
) -> Result<(), String> {
    let url = validated_url(&url)?;
    if let Some(old) = app.get_webview(LABEL) {
        old.close().map_err(|e| e.to_string())?;
    }
    let window = app.get_window("main").ok_or("找不到主窗口")?;
    let handle = app.clone();
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(url))
        .on_navigation(|url| validated_url(url.as_str()).is_ok())
        .on_new_window(move |url, _| {
            if validated_url(url.as_str()).is_ok() {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(webview) = handle.get_webview(LABEL) {
                        let _ = webview.navigate(url);
                    }
                });
            }
            NewWindowResponse::Deny
        });
    let view = window
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|e| e.to_string())?;
    view.hide().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    watch_webview_events(&view, app, generation);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn web_panel_bounds(
    app: tauri::AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
) -> Result<(), String> {
    let Some(view) = app.get_webview(LABEL) else {
        return Ok(());
    };
    if ![x, y, width, height].iter().all(|n| n.is_finite()) || width < 0.0 || height < 0.0 {
        return Err("无效的网页视图尺寸".into());
    }
    if visible && width > 0.0 && height > 0.0 {
        view.set_position(LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
        view.set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())?;
    } else {
        view.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn web_panel_close(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(view) = app.get_webview(LABEL) {
        view.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn watch_webview_events(view: &tauri::Webview, app: tauri::AppHandle, generation: String) {
    use tauri::Emitter;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
    };
    use webview2_com::{AcceleratorKeyPressedEventHandler, NavigationCompletedEventHandler};
    if let Err(error) = view.with_webview(move |platform| {
        let apply = || -> windows::core::Result<()> {
            let controller = platform.controller();
            let core = unsafe { controller.CoreWebView2()? };
            let status_app = app.clone();
            let status_generation = generation.clone();
            let handler =
                NavigationCompletedEventHandler::create(Box::new(move |_sender, args| {
                    if let Some(args) = args {
                        let mut success = windows::core::BOOL::default();
                        if unsafe { args.IsSuccess(&mut success) }.is_ok() {
                            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;
                            if success.as_bool()
                                || (unsafe { args.WebErrorStatus(&mut status) }.is_ok()
                                    && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED)
                            {
                                let _ = status_app.emit_to(
                                    "main",
                                    "web-panel-load-status",
                                    (status_generation.clone(), success.as_bool()),
                                );
                            }
                        }
                    }
                    Ok(())
                }));
            let mut token = 0;
            unsafe {
                core.add_NavigationCompleted(&handler, &mut token)?;
            }
            let key_handler =
                AcceleratorKeyPressedEventHandler::create(Box::new(move |_sender, args| {
                    if let Some(args) = args {
                        let mut key = 0;
                        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                        if unsafe { args.VirtualKey(&mut key) }.is_ok()
                            && unsafe { args.KeyEventKind(&mut kind) }.is_ok()
                            && key == 27
                            && kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                        {
                            unsafe {
                                args.SetHandled(true)?;
                            }
                            let _ =
                                app.emit_to("main", "web-panel-close-request", generation.clone());
                        }
                    }
                    Ok(())
                }));
            let mut key_token = 0;
            unsafe {
                controller.add_AcceleratorKeyPressed(&key_handler, &mut key_token)?;
            }
            Ok(())
        };
        if let Err(error) = apply() {
            tracing::warn!("网页事件监听失败：{error}");
        }
    }) {
        tracing::warn!("无法访问网页视图：{error}");
    }
}
