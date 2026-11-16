use tauri::{
    LogicalPosition, LogicalSize, Manager, WebviewUrl,
    webview::{NewWindowResponse, WebviewBuilder},
};
use tauri_plugin_opener::OpenerExt;

pub(crate) const LABEL: &str = "session-link-web";

pub(crate) fn external_url(raw: &str) -> Result<tauri::Url, String> {
    let url = tauri::Url::parse(raw).map_err(|_| "无效的网址".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("仅支持 HTTP 和 HTTPS 网址".into());
    }
    Ok(url)
}

pub(crate) fn validated_url(raw: &str) -> Result<tauri::Url, String> {
    // about:blank 仅作为可视浏览器后端的初始页（契约白名单允许）。
    if raw == "about:blank" {
        return tauri::Url::parse(raw).map_err(|_| "无效的网址".to_string());
    }
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

/// 打开（或复用）面板 webview 并导航到 `url`：已存在时不重建，仅导航。
/// 供 `web_panel_open` 命令与可视浏览器后端的 `ensure_webview` 共用。
pub(crate) async fn create_panel_webview(
    app: &tauri::AppHandle,
    url: &str,
) -> Result<tauri::Webview, String> {
    let parsed = validated_url(url)?;
    if let Some(view) = app.get_webview(LABEL) {
        // 目标与当前页一致时不再导航：前端挂载与 agent 首开回环会对同一 URL
        // 重复调用，重复导航会造成双重加载。
        #[cfg(windows)]
        if crate::browser_backend::panel_current_url(app).as_deref() == Some(url) {
            return Ok(view);
        }
        view.navigate(parsed).map_err(|e| e.to_string())?;
        return Ok(view);
    }
    let window = app.get_window("main").ok_or("找不到主窗口")?;
    let handle = app.clone();
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(parsed))
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
    crate::browser_backend::register_panel_listeners(&view, app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn web_panel_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
    create_panel_webview(&app, &url).await.map(|_| ())
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

#[tauri::command]
#[specta::specta]
pub async fn web_panel_history(app: tauri::AppHandle, action: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::browser_backend::panel_history(&app, &action).await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, action);
        Err("网页面板历史导航仅支持 Windows".into())
    }
}
