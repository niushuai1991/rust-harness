//! 「网页」面板 WebView2 的可视浏览器后端：与 headless CDP 后端共用命令契约。
//!
//! 面板 WebView2 既是用户浏览的界面，也是模型 `browser_*` 工具操作的浏览器（人机
//! 共用同一个实例）。执行路径：tokio 任务 → 主线程 `with_webview` → 进程内 CDP
//! （`ICoreWebView2::CallDevToolsProtocolMethod`）→ oneshot 回传；导航等待依赖
//! `NavigationCompleted` 事件转发的 watch 计数。webview 生命周期归 UI 层所有：
//! `close` 与空闲回收只清会话状态（ref 映射），不销毁用户正在查看的页面。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use base64::Engine;
use harness_browser::backend::{
    BrowserBackend, center, describe_exception, png_size, translate_key,
};
use harness_browser::command::{
    BrowserCommand, CommandOutcome, ElementInfo, ScreenshotInfo, SnapshotView, is_allowed_url,
};
use harness_browser::snapshot::{COLLECT_JS, LOCATE_JS, Located, RawElement};
use harness_contracts::{AppError, AppResult};
use serde_json::{Value, json};
use tauri::Manager;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const CLICK_NAVIGATION_WINDOW: Duration = Duration::from_millis(800);

/// 共享桥：面板 webview 的导航信号、当前 URL 与 ref→locator 会话状态（跨后端实例持久）。
pub struct BrowserBridge {
    app: tauri::AppHandle,
    nav_tx: tokio::sync::watch::Sender<u64>,
    /// ref → CSS locator 映射；仅存在于 Rust 侧，页面脚本不可触碰。
    refs: Mutex<HashMap<String, String>>,
    /// 最近一次导航完成时的页面 URL（`NavigationCompleted` 时刷新）。
    current: Mutex<Option<String>>,
}

pub fn new_bridge(app: tauri::AppHandle) -> Arc<BrowserBridge> {
    let (nav_tx, _) = tokio::sync::watch::channel(0);
    Arc::new(BrowserBridge {
        app,
        nav_tx,
        refs: Mutex::new(HashMap::new()),
        current: Mutex::new(None),
    })
}

impl BrowserBridge {
    fn webview(&self) -> Option<tauri::Webview> {
        self.app.get_webview(crate::web_panel::LABEL)
    }

    fn nav_current(&self) -> u64 {
        self.nav_tx.borrow().clone()
    }

    /// 等待下一次导航完成（`NavigationCompleted`）；超时返回 false，不视为失败。
    async fn wait_navigation(&self, before: u64, timeout: Duration) -> bool {
        let mut rx = self.nav_tx.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if *rx.borrow_and_update() > before {
                return true;
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => continue,
                _ => return false,
            }
        }
    }

    /// 确保面板 webview 存在；不存在时以 `initial` 创建并通知前端弹出抽屉。
    async fn ensure_webview(&self, initial: Option<&str>) -> AppResult<tauri::Webview> {
        if let Some(view) = self.webview() {
            return Ok(view);
        }
        let url = initial.unwrap_or("about:blank").to_string();
        let view = crate::web_panel::create_panel_webview(&self.app, &url)
            .await
            .map_err(|e| AppError::new("WEBVIEW_UNAVAILABLE", e))?;
        // 预置当前 URL：后端随后的 Page.navigate 完成前，前端经 agent-open 弹出
        // 面板时不会再对同一 URL 发起第二次导航。
        *self.current.lock().unwrap() = Some(url.clone());
        let _ = tauri::Emitter::emit_to(&self.app, "main", "web-panel-agent-open", url);
        Ok(view)
    }

    /// 在面板 webview 上发起一次进程内 CDP 调用并等待完成。
    async fn cdp(&self, method: &str, params: Value, timeout: Duration) -> AppResult<Value> {
        let view = self
            .webview()
            .ok_or_else(|| AppError::new("WEBVIEW_UNAVAILABLE", "网页面板尚未创建"))?;
        self.cdp_on(&view, method, params, timeout).await
    }

    async fn cdp_on(
        &self,
        view: &tauri::Webview,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> AppResult<Value> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        // oneshot 发送端只有一个会真正持有：完成回调先到则用回调，COM 调用同步失败则用兜底。
        let slot = Arc::new(Mutex::new(Some(tx)));
        let method = method.to_string();
        let params = params.to_string();
        let handler_slot = slot.clone();
        view.with_webview(move |platform| {
            let run = || -> windows::core::Result<()> {
                let controller = platform.controller();
                let core = unsafe { controller.CoreWebView2()? };
                let handler = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(
                    Box::new(move |result, result_json| {
                        if let Some(tx) = handler_slot.lock().unwrap().take() {
                            let _ = match result {
                                Ok(()) => tx.send(Ok(result_json.to_string())),
                                Err(error) => tx.send(Err(format!("CDP 调用失败：{error}"))),
                            };
                        }
                        Ok(())
                    }),
                );
                unsafe {
                    // 签名顺序：methodname、parametersasjson、handler（回调在最后）。
                    core.CallDevToolsProtocolMethod(
                        &windows::core::HSTRING::from(&method),
                        &windows::core::HSTRING::from(&params),
                        &handler,
                    )?;
                }
                Ok(())
            };
            if let Err(error) = run() {
                if let Some(tx) = slot.lock().unwrap().take() {
                    let _ = tx.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|e| AppError::new("WEBVIEW_UNAVAILABLE", e.to_string()))?;
        let result = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| AppError::new("WEBVIEW_TIMEOUT", "CDP 调用超时"))?
            .map_err(|_| AppError::new("WEBVIEW_CDP", "CDP 调用通道已关闭"))?
            .map_err(|e| AppError::new("WEBVIEW_CDP", e))?;
        serde_json::from_str(&result)
            .map_err(|e| AppError::new("WEBVIEW_CDP", format!("CDP 返回解析失败：{e}")))
    }
}

/// 面板 webview 当前页面 URL（供去重导航判断；无面板或桥未就绪时为 None）。
pub(crate) fn panel_current_url(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<Arc<BrowserBridge>>()
        .and_then(|bridge| bridge.current.lock().unwrap().clone())
}

/// 导航白名单（契约层 + 应用内部地址防护）；`about:blank` 仅用于面板初始化。
fn check_url(url: &str) -> AppResult<()> {
    if !is_allowed_url(url) {
        return Err(AppError::new(
            "URL_FORBIDDEN",
            format!("仅允许 http/https/about:blank 导航地址，已拒绝：{url}"),
        ));
    }
    if url != "about:blank" {
        crate::web_panel::validated_url(url).map_err(|e| AppError::new("URL_FORBIDDEN", e))?;
    }
    Ok(())
}

/// 面板历史导航动作 → 命令契约。
fn history_command(action: &str) -> AppResult<BrowserCommand> {
    Ok(match action {
        "back" => BrowserCommand::Back,
        "forward" => BrowserCommand::Forward,
        "reload" => BrowserCommand::Reload,
        other => {
            return Err(AppError::new(
                "TOOL_ARGUMENTS",
                format!("无效的面板导航动作：{other}"),
            ));
        }
    })
}

/// 面板工具栏的后退/前进/刷新：与模型命令共用同一后端路径（ref 映射一致失效）。
pub(crate) async fn panel_history(app: &tauri::AppHandle, action: &str) -> Result<(), String> {
    let Some(backend) = app.try_state::<Arc<WebView2Backend>>() else {
        return Err("可视浏览器后端未就绪".into());
    };
    if app.get_webview(crate::web_panel::LABEL).is_none() {
        return Err("网页面板尚未打开".into());
    }
    let command = history_command(action).map_err(|e| e.message)?;
    backend
        .execute(command)
        .await
        .map(|_| ())
        .map_err(|e| e.message)
}

/// 注册面板 webview 的监听：加载状态/当前 URL、前进后退可用性、ESC 关闭；
/// 同时禁用右键菜单与浏览器快捷键（同主窗口处理，避免 F5 绕过 ref 失效逻辑）。
pub(crate) fn register_panel_listeners(view: &tauri::Webview, app: &tauri::AppHandle) {
    let Some(bridge) = app
        .try_state::<Arc<BrowserBridge>>()
        .map(|s| s.inner().clone())
    else {
        tracing::warn!("可视浏览器桥未就绪，跳过面板事件监听");
        return;
    };
    let result = view.with_webview(move |platform| {
        let apply = || -> windows::core::Result<()> {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
                COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
            };
            use windows::core::Interface;

            let controller = platform.controller();
            let core = unsafe { controller.CoreWebView2()? };
            let settings = unsafe { core.Settings()? };
            unsafe {
                settings.SetAreDefaultContextMenusEnabled(false)?;
            }
            if let Ok(settings3) =
                settings
                    .cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3>()
            {
                unsafe {
                    settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
                }
            }
            // 导航完成：推进 watch 计数（等待器）、清 ref 映射、下发加载状态与当前 URL。
            let nav_bridge = bridge.clone();
            let nav_handler = webview2_com::NavigationCompletedEventHandler::create(Box::new(
                move |core, args| {
                    if let Some(args) = args {
                        let mut success = windows::core::BOOL::default();
                        let ok =
                            unsafe { args.IsSuccess(&mut success) }.is_ok() && success.as_bool();
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;
                        let report = ok
                            || (unsafe { args.WebErrorStatus(&mut status) }.is_ok()
                                && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED);
                        if report {
                            let next = nav_bridge.nav_current() + 1;
                            let _ = nav_bridge.nav_tx.send(next);
                            nav_bridge.refs.lock().unwrap().clear();
                            let url = core.as_ref().and_then(source_url);
                            *nav_bridge.current.lock().unwrap() = url.clone();
                            let _ = tauri::Emitter::emit_to(
                                &nav_bridge.app,
                                "main",
                                "web-panel-load-status",
                                (ok, url),
                            );
                        }
                    }
                    Ok(())
                },
            ));
            let mut nav_token = 0;
            unsafe {
                core.add_NavigationCompleted(&nav_handler, &mut nav_token)?;
            }
            // 历史变化：下发前进/后退可用性（工具栏禁用态）。
            let history_app = bridge.app.clone();
            let history_handler =
                webview2_com::HistoryChangedEventHandler::create(Box::new(move |core, _| {
                    if let Some(core) = core {
                        let (mut back, mut forward) = (
                            windows::core::BOOL::default(),
                            windows::core::BOOL::default(),
                        );
                        unsafe {
                            let _ = core.CanGoBack(&mut back);
                            let _ = core.CanGoForward(&mut forward);
                        }
                        let _ = tauri::Emitter::emit_to(
                            &history_app,
                            "main",
                            "web-panel-history",
                            (back.as_bool(), forward.as_bool()),
                        );
                    }
                    Ok(())
                }));
            let mut history_token = 0;
            unsafe {
                core.add_HistoryChanged(&history_handler, &mut history_token)?;
            }
            // ESC 关闭面板（全局单实例语义，事件不再携带代际）。
            let escape_app = bridge.app.clone();
            let escape_handler = webview2_com::AcceleratorKeyPressedEventHandler::create(Box::new(
                move |_core, args| {
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
                            let _ = tauri::Emitter::emit_to(
                                &escape_app,
                                "main",
                                "web-panel-close-request",
                                (),
                            );
                        }
                    }
                    Ok(())
                },
            ));
            let mut escape_token = 0;
            unsafe {
                controller.add_AcceleratorKeyPressed(&escape_handler, &mut escape_token)?;
            }
            Ok(())
        };
        if let Err(error) = apply() {
            tracing::warn!("面板事件监听注册失败：{error}");
        }
    });
    if let Err(error) = result {
        tracing::warn!("无法访问面板 webview：{error}");
    }
}

/// 读取当前页面 URL（COM 分配的字符串用后须释放）。
fn source_url(
    core: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
) -> Option<String> {
    unsafe {
        let mut href = windows::core::PWSTR::null();
        core.Source(&mut href).ok()?;
        if href.is_null() {
            return None;
        }
        let url = href.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(href.as_ptr().cast()));
        url
    }
}

/// WebView2 可视后端：`execute` 内部经 [`BrowserBridge`] 在主线程驱动面板 webview。
pub struct WebView2Backend {
    bridge: Arc<BrowserBridge>,
    screenshots_dir: PathBuf,
    counter: AtomicU64,
}

impl WebView2Backend {
    fn new(bridge: Arc<BrowserBridge>, screenshots_dir: PathBuf) -> Self {
        Self {
            bridge,
            screenshots_dir,
            counter: AtomicU64::new(0),
        }
    }

    async fn page_info(&self) -> AppResult<(String, String)> {
        let value = self
            .bridge
            .cdp(
                "Runtime.evaluate",
                json!({
                    "expression": "JSON.stringify({url: location.href, title: document.title})",
                    "returnByValue": true
                }),
                CALL_TIMEOUT,
            )
            .await?;
        let parsed: Value = value["result"]["value"]
            .as_str()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or(Value::Null);
        Ok((
            parsed["url"].as_str().unwrap_or_default().to_string(),
            parsed["title"].as_str().unwrap_or_default().to_string(),
        ))
    }

    async fn collect(&self) -> AppResult<SnapshotView> {
        let value = self
            .bridge
            .cdp(
                "Runtime.evaluate",
                json!({"expression": COLLECT_JS, "returnByValue": true}),
                CALL_TIMEOUT,
            )
            .await?;
        if let Some(exception) = value.get("exceptionDetails") {
            return Err(AppError::new(
                "SNAPSHOT_FAILED",
                describe_exception(exception),
            ));
        }
        let text = value["result"]["value"]
            .as_str()
            .ok_or_else(|| AppError::new("SNAPSHOT_FAILED", "页面快照收集返回为空"))?;
        let raw: Vec<RawElement> = serde_json::from_str(text)
            .map_err(|e| AppError::new("SNAPSHOT_FAILED", format!("快照解析失败：{e}")))?;
        let (url, title) = self.page_info().await?;
        *self.bridge.refs.lock().unwrap() = raw
            .iter()
            .map(|e| (e.r#ref.clone(), e.locator.clone()))
            .collect();
        let elements = raw
            .into_iter()
            .map(|e| ElementInfo {
                r#ref: e.r#ref,
                role: e.role,
                tag: e.tag,
                name: e.name,
                value: e.value,
            })
            .collect();
        Ok(SnapshotView {
            url,
            title,
            elements,
        })
    }

    async fn locate(&self, reference: &str) -> AppResult<Located> {
        let locator = {
            let refs = self.bridge.refs.lock().unwrap();
            refs.get(reference).cloned()
        };
        let Some(locator) = locator else {
            return Err(AppError::new(
                "REF_STALE",
                format!("未知或已失效的元素引用 {reference}；请先执行 browser_snapshot"),
            ));
        };
        let encoded = serde_json::to_string(&locator)
            .map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
        let expression = format!("{LOCATE_JS}({encoded})");
        let value = self
            .bridge
            .cdp(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
                CALL_TIMEOUT,
            )
            .await?;
        if let Some(exception) = value.get("exceptionDetails") {
            return Err(AppError::new(
                "LOCATE_FAILED",
                describe_exception(exception),
            ));
        }
        let located: Option<Located> = value["result"]["value"]
            .as_str()
            .and_then(|t| serde_json::from_str(t).ok());
        located.ok_or_else(|| {
            AppError::new(
                "REF_STALE",
                format!("元素 {reference} 已不在当前页面；请重新执行 browser_snapshot"),
            )
        })
    }

    async fn navigate(&self, url: &str) -> AppResult<CommandOutcome> {
        check_url(url)?;
        let view = self.bridge.ensure_webview(Some(url)).await?;
        let before = self.bridge.nav_current();
        let result = self
            .bridge
            .cdp_on(&view, "Page.navigate", json!({"url": url}), CALL_TIMEOUT)
            .await?;
        if let Some(error) = result["errorText"].as_str().filter(|s| !s.is_empty()) {
            return Err(AppError::new("NAVIGATION_FAILED", error.to_string()));
        }
        self.bridge.wait_navigation(before, LOAD_TIMEOUT).await;
        self.bridge.refs.lock().unwrap().clear();
        let (current, title) = self.page_info().await?;
        Ok(CommandOutcome {
            summary: format!("已打开 {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn go_history(&self, offset: i64) -> AppResult<CommandOutcome> {
        let label = if offset < 0 {
            "已后退到"
        } else {
            "已前进到"
        };
        let view = self.bridge.ensure_webview(None).await?;
        let before = self.bridge.nav_current();
        let history = self
            .bridge
            .cdp_on(&view, "Page.getNavigationHistory", json!({}), CALL_TIMEOUT)
            .await?;
        let index = history["currentIndex"].as_i64().unwrap_or(0);
        let entries = history["entries"].as_array();
        let last = entries.map_or(0, |e| e.len() as i64 - 1);
        let target = index + offset;
        if target < 0 || target > last {
            return Err(AppError::new(
                "NAVIGATION_END",
                if offset < 0 {
                    "没有可后退的历史记录"
                } else {
                    "没有可前进的历史记录"
                },
            ));
        }
        let entry_id = entries
            .and_then(|e| e[target as usize]["id"].as_i64())
            .ok_or_else(|| AppError::new("NAVIGATION_END", "历史记录不可用"))?;
        self.bridge
            .cdp_on(
                &view,
                "Page.navigateToHistoryEntry",
                json!({"entryId": entry_id}),
                CALL_TIMEOUT,
            )
            .await?;
        self.bridge.wait_navigation(before, LOAD_TIMEOUT).await;
        self.bridge.refs.lock().unwrap().clear();
        let (current, title) = self.page_info().await?;
        Ok(CommandOutcome {
            summary: format!("{label} {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn reload(&self) -> AppResult<CommandOutcome> {
        let view = self.bridge.ensure_webview(None).await?;
        let before = self.bridge.nav_current();
        self.bridge
            .cdp_on(
                &view,
                "Page.reload",
                json!({"ignoreCache": false}),
                CALL_TIMEOUT,
            )
            .await?;
        self.bridge.wait_navigation(before, LOAD_TIMEOUT).await;
        self.bridge.refs.lock().unwrap().clear();
        let (current, title) = self.page_info().await?;
        Ok(CommandOutcome {
            summary: format!("已刷新 {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn click(&self, reference: &str) -> AppResult<CommandOutcome> {
        self.bridge.ensure_webview(None).await?;
        let target = self.locate(reference).await?;
        let before = self.bridge.nav_current();
        dispatch_click(&self.bridge, center(target)).await?;
        // 点击可能触发导航：短暂窗口内命中则使 ref 映射失效。
        if self
            .bridge
            .wait_navigation(before, CLICK_NAVIGATION_WINDOW)
            .await
        {
            self.bridge.refs.lock().unwrap().clear();
        }
        let snapshot = self.collect().await?;
        Ok(CommandOutcome {
            summary: format!("已点击 {reference}"),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn type_text(
        &self,
        reference: &str,
        text: &str,
        submit: bool,
    ) -> AppResult<CommandOutcome> {
        self.bridge.ensure_webview(None).await?;
        let target = self.locate(reference).await?;
        dispatch_click(&self.bridge, center(target)).await?;
        self.bridge
            .cdp("Input.insertText", json!({"text": text}), CALL_TIMEOUT)
            .await?;
        if submit {
            press_key(&self.bridge, "Enter").await?;
        }
        let snapshot = self.collect().await?;
        Ok(CommandOutcome {
            summary: format!("已在 {reference} 输入 {} 字符", text.chars().count()),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn press(&self, key: &str) -> AppResult<CommandOutcome> {
        self.bridge.ensure_webview(None).await?;
        press_key(&self.bridge, key).await?;
        let snapshot = self.collect().await?;
        Ok(CommandOutcome {
            summary: format!("已发送按键 {key}"),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn screenshot(&self, full_page: bool) -> AppResult<CommandOutcome> {
        self.bridge.ensure_webview(None).await?;
        let params = if full_page {
            let metrics = self
                .bridge
                .cdp("Page.getLayoutMetrics", json!({}), SCREENSHOT_TIMEOUT)
                .await?;
            let content = &metrics["cssContentSize"];
            json!({
                "format": "png",
                "captureBeyondViewport": true,
                "clip": {"x": 0, "y": 0, "width": content["width"], "height": content["height"], "scale": 1}
            })
        } else {
            json!({"format": "png", "captureBeyondViewport": false})
        };
        let value = self
            .bridge
            .cdp("Page.captureScreenshot", params, SCREENSHOT_TIMEOUT)
            .await?;
        let data = value["data"]
            .as_str()
            .ok_or_else(|| AppError::new("SCREENSHOT_FAILED", "截图数据为空"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| AppError::new("SCREENSHOT_FAILED", format!("截图解码失败：{e}")))?;
        let Some((width, height)) = png_size(&bytes) else {
            return Err(AppError::new("SCREENSHOT_FAILED", "截图不是合法 PNG"));
        };
        std::fs::create_dir_all(&self.screenshots_dir)
            .map_err(|e| AppError::new("SCREENSHOT_FAILED", format!("创建截图目录失败：{e}")))?;
        let stamp = chrono::Utc::now().timestamp_millis();
        let path = self.screenshots_dir.join(format!(
            "browser-{stamp}-{}.png",
            self.counter.fetch_add(1, Ordering::Relaxed)
        ));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| AppError::new("SCREENSHOT_FAILED", format!("写入截图失败：{e}")))?;
        let path = path.display().to_string();
        Ok(CommandOutcome {
            summary: format!("截图已保存：{path}（{width}x{height}）"),
            screenshot: Some(ScreenshotInfo {
                path,
                width,
                height,
            }),
            ..Default::default()
        })
    }

    async fn evaluate(&self, expression: &str) -> AppResult<CommandOutcome> {
        self.bridge.ensure_webview(None).await?;
        let value = self
            .bridge
            .cdp(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
                CALL_TIMEOUT,
            )
            .await?;
        if let Some(exception) = value.get("exceptionDetails") {
            return Err(AppError::new(
                "EVALUATE_FAILED",
                describe_exception(exception),
            ));
        }
        Ok(CommandOutcome {
            summary: "表达式已执行".into(),
            value: Some(value["result"]["value"].clone()),
            ..Default::default()
        })
    }
}

#[async_trait::async_trait]
impl BrowserBackend for WebView2Backend {
    async fn execute(&self, command: BrowserCommand) -> AppResult<CommandOutcome> {
        match command {
            BrowserCommand::Navigate { url } => self.navigate(&url).await,
            BrowserCommand::Back => self.go_history(-1).await,
            BrowserCommand::Forward => self.go_history(1).await,
            BrowserCommand::Reload => self.reload().await,
            BrowserCommand::Snapshot => {
                let snapshot = self.collect().await?;
                let summary = format!("共 {} 个可交互元素", snapshot.elements.len());
                Ok(CommandOutcome {
                    summary,
                    snapshot: Some(snapshot),
                    ..Default::default()
                })
            }
            BrowserCommand::Click { r#ref } => self.click(&r#ref).await,
            BrowserCommand::Type {
                r#ref,
                text,
                submit,
            } => self.type_text(&r#ref, &text, submit).await,
            BrowserCommand::Press { key } => self.press(&key).await,
            BrowserCommand::Screenshot { full_page } => self.screenshot(full_page).await,
            BrowserCommand::Evaluate { expression } => self.evaluate(&expression).await,
            BrowserCommand::Close => {
                self.bridge.refs.lock().unwrap().clear();
                Ok(CommandOutcome {
                    summary: "浏览器会话已关闭".into(),
                    ..Default::default()
                })
            }
        }
    }

    /// 面板 webview 归 UI 层所有：仅清会话状态，不销毁页面（空闲回收同样安全）。
    async fn close(&self) -> AppResult<()> {
        self.bridge.refs.lock().unwrap().clear();
        Ok(())
    }
}

/// 工厂：应用启动时构造一次，后续 `create` 始终返回同一实例（面板 webview 持久）。
pub struct WebView2Factory {
    backend: Arc<WebView2Backend>,
}

impl WebView2Factory {
    pub fn new(bridge: Arc<BrowserBridge>, screenshots_dir: PathBuf) -> Self {
        Self {
            backend: Arc::new(WebView2Backend::new(bridge, screenshots_dir)),
        }
    }

    /// 供面板工具栏命令直接访问后端（与模型命令共用实例）。
    pub fn backend(&self) -> Arc<WebView2Backend> {
        self.backend.clone()
    }
}

impl harness_browser::BackendFactory for WebView2Factory {
    fn create(&self) -> futures::future::BoxFuture<'_, AppResult<Arc<dyn BrowserBackend>>> {
        let backend = self.backend.clone();
        Box::pin(async move { Ok(backend as Arc<dyn BrowserBackend>) })
    }
}

async fn dispatch_click(bridge: &BrowserBridge, (x, y): (f64, f64)) -> AppResult<()> {
    bridge
        .cdp(
            "Input.dispatchMouseEvent",
            json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1}),
            CALL_TIMEOUT,
        )
        .await?;
    bridge
        .cdp(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1}),
            CALL_TIMEOUT,
        )
        .await?;
    Ok(())
}

async fn press_key(bridge: &BrowserBridge, key: &str) -> AppResult<()> {
    let (key_name, code, vk) = translate_key(key).ok_or_else(|| {
        AppError::new(
            "UNSUPPORTED_KEY",
            format!("不支持的按键：{key}（支持 Enter/Tab/Escape/方向键等）"),
        )
    })?;
    let mut params = json!({
        "key": key_name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk
    });
    params["type"] = json!("keyDown");
    bridge
        .cdp("Input.dispatchKeyEvent", params.clone(), CALL_TIMEOUT)
        .await?;
    params["type"] = json!("keyUp");
    bridge
        .cdp("Input.dispatchKeyEvent", params, CALL_TIMEOUT)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_guard_rejects_disallowed_schemes_and_internal_addresses() {
        assert!(check_url("http://localhost:5173/").is_ok());
        assert!(check_url("https://example.com/x").is_ok());
        assert!(check_url("about:blank").is_ok());
        for url in [
            "file:///C:/secret",
            "javascript:alert(1)",
            "data:text/html,x",
            "http://tauri.localhost/",
            "http://127.0.0.1:1420/",
        ] {
            let err = check_url(url).unwrap_err();
            assert_eq!(err.code, "URL_FORBIDDEN", "{url}");
        }
    }

    #[test]
    fn history_command_maps_three_actions_only() {
        assert_eq!(history_command("back").unwrap(), BrowserCommand::Back);
        assert_eq!(history_command("forward").unwrap(), BrowserCommand::Forward);
        assert_eq!(history_command("reload").unwrap(), BrowserCommand::Reload);
        assert_eq!(history_command("home").unwrap_err().code, "TOOL_ARGUMENTS");
    }
}
