//! `BrowserBackend` 抽象与 headless CDP 实现。
use crate::cdp::{CdpConnection, wait_for};
use crate::command::{
    BrowserCommand, CommandOutcome, ElementInfo, ScreenshotInfo, SnapshotView, is_allowed_url,
};
use crate::launcher::{LaunchedBrowser, find_executable};
use crate::snapshot::{COLLECT_JS, LOCATE_JS, Located, RawElement};
use async_trait::async_trait;
use base64::Engine;
use harness_contracts::{AppError, AppResult};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

/// 后端抽象：执行统一命令契约；未来 WebView2 可视后端实现同一接口。
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    async fn execute(&self, command: BrowserCommand) -> AppResult<CommandOutcome>;
    /// 终止浏览器进程并释放会话资源。
    async fn close(&self) -> AppResult<()>;
}

struct LiveSession {
    browser: LaunchedBrowser,
    cdp: CdpConnection,
    /// ref → CSS locator 映射；仅存在于 Rust 侧，页面脚本不可触碰。
    refs: HashMap<String, String>,
}

/// headless CDP 后端：首次命令时懒启动浏览器，单页面会话，`close` 后可再次懒启动。
pub struct CdpBackend {
    session: tokio::sync::Mutex<Option<LiveSession>>,
    screenshots_dir: PathBuf,
    counter: AtomicU64,
}

impl CdpBackend {
    pub fn new(screenshots_dir: PathBuf) -> Self {
        Self {
            session: tokio::sync::Mutex::new(None),
            screenshots_dir,
            counter: AtomicU64::new(0),
        }
    }

    async fn ensure_session<'a>(
        &self,
        slot: &'a mut Option<LiveSession>,
    ) -> AppResult<&'a mut LiveSession> {
        if slot.is_none() {
            let executable = find_executable(None)?;
            let browser = LaunchedBrowser::launch(&executable).await?;
            let cdp = attach_page(&browser).await?;
            tracing::info!(port = browser.debug_port, "headless 浏览器已启动");
            *slot = Some(LiveSession {
                browser,
                cdp,
                refs: HashMap::new(),
            });
        }
        Ok(slot.as_mut().expect("session ensured"))
    }

    async fn navigate(&self, session: &mut LiveSession, url: &str) -> AppResult<CommandOutcome> {
        if !is_allowed_url(url) {
            return Err(AppError::new(
                "URL_FORBIDDEN",
                format!("仅允许 http/https/about:blank 导航地址，已拒绝：{url}"),
            ));
        }
        let mut events = session.cdp.subscribe();
        let result = session
            .cdp
            .call("Page.navigate", json!({"url": url}))
            .await?;
        if let Some(error) = result["errorText"].as_str().filter(|s| !s.is_empty()) {
            return Err(AppError::new("NAVIGATION_FAILED", error.to_string()));
        }
        wait_for(&mut events, "Page.loadEventFired", Duration::from_secs(15)).await;
        session.refs.clear();
        let (current, title) = page_info(session).await?;
        Ok(CommandOutcome {
            summary: format!("已打开 {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn back(&self, session: &mut LiveSession) -> AppResult<CommandOutcome> {
        let mut events = session.cdp.subscribe();
        let history = session
            .cdp
            .call("Page.getNavigationHistory", json!({}))
            .await?;
        let index = history["currentIndex"].as_i64().unwrap_or(0);
        if index <= 0 {
            return Err(AppError::new("NAVIGATION_END", "没有可后退的历史记录"));
        }
        let entry_id = history["entries"][index as usize - 1]["id"]
            .as_i64()
            .ok_or_else(|| AppError::new("NAVIGATION_END", "历史记录不可用"))?;
        session
            .cdp
            .call("Page.navigateToHistoryEntry", json!({"entryId": entry_id}))
            .await?;
        wait_for(&mut events, "Page.loadEventFired", Duration::from_secs(10)).await;
        session.refs.clear();
        let (current, title) = page_info(session).await?;
        Ok(CommandOutcome {
            summary: format!("已后退到 {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn reload(&self, session: &mut LiveSession) -> AppResult<CommandOutcome> {
        let mut events = session.cdp.subscribe();
        session
            .cdp
            .call("Page.reload", json!({"ignoreCache": false}))
            .await?;
        wait_for(&mut events, "Page.loadEventFired", Duration::from_secs(15)).await;
        session.refs.clear();
        let (current, title) = page_info(session).await?;
        Ok(CommandOutcome {
            summary: format!("已刷新 {current}（{title}）"),
            ..Default::default()
        })
    }

    async fn collect(&self, session: &mut LiveSession) -> AppResult<SnapshotView> {
        let value = session
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": COLLECT_JS, "returnByValue": true}),
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
        let (url, title) = page_info(session).await?;
        session.refs = raw
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

    async fn locate(&self, session: &LiveSession, reference: &str) -> AppResult<Located> {
        let Some(locator) = session.refs.get(reference) else {
            return Err(AppError::new(
                "REF_STALE",
                format!("未知或已失效的元素引用 {reference}；请先执行 browser_snapshot"),
            ));
        };
        let encoded =
            serde_json::to_string(locator).map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
        let expression = format!("{LOCATE_JS}({encoded})");
        let value = session
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
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

    async fn click(&self, session: &mut LiveSession, reference: &str) -> AppResult<CommandOutcome> {
        let target = self.locate(session, reference).await?;
        let mut events = session.cdp.subscribe();
        dispatch_click(&session.cdp, center(target)).await?;
        // 点击可能触发导航：短暂等待 load 事件，命中则使 ref 映射失效。
        if wait_for(
            &mut events,
            "Page.loadEventFired",
            Duration::from_millis(800),
        )
        .await
        {
            session.refs.clear();
        }
        let snapshot = self.collect(session).await?;
        Ok(CommandOutcome {
            summary: format!("已点击 {reference}"),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn type_text(
        &self,
        session: &mut LiveSession,
        reference: &str,
        text: &str,
        submit: bool,
    ) -> AppResult<CommandOutcome> {
        let target = self.locate(session, reference).await?;
        dispatch_click(&session.cdp, center(target)).await?;
        session
            .cdp
            .call("Input.insertText", json!({"text": text}))
            .await?;
        if submit {
            press_key(&session.cdp, "Enter").await?;
        }
        let snapshot = self.collect(session).await?;
        Ok(CommandOutcome {
            summary: format!("已在 {reference} 输入 {} 字符", text.chars().count()),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn press(&self, session: &mut LiveSession, key: &str) -> AppResult<CommandOutcome> {
        press_key(&session.cdp, key).await?;
        let snapshot = self.collect(session).await?;
        Ok(CommandOutcome {
            summary: format!("已发送按键 {key}"),
            snapshot: Some(snapshot),
            ..Default::default()
        })
    }

    async fn screenshot(
        &self,
        session: &mut LiveSession,
        full_page: bool,
    ) -> AppResult<CommandOutcome> {
        let params = if full_page {
            let metrics = session.cdp.call("Page.getLayoutMetrics", json!({})).await?;
            let content = &metrics["cssContentSize"];
            json!({
                "format": "png",
                "captureBeyondViewport": true,
                "clip": {"x": 0, "y": 0, "width": content["width"], "height": content["height"], "scale": 1}
            })
        } else {
            json!({"format": "png", "captureBeyondViewport": false})
        };
        let value = session
            .cdp
            .call_timeout("Page.captureScreenshot", params, Duration::from_secs(60))
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

    async fn evaluate(&self, session: &LiveSession, expression: &str) -> AppResult<CommandOutcome> {
        let value = session
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
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

#[async_trait]
impl BrowserBackend for CdpBackend {
    async fn execute(&self, command: BrowserCommand) -> AppResult<CommandOutcome> {
        let mut guard = self.session.lock().await;
        let session = self.ensure_session(&mut guard).await?;
        match command {
            BrowserCommand::Navigate { url } => self.navigate(session, &url).await,
            BrowserCommand::Back => self.back(session).await,
            BrowserCommand::Reload => self.reload(session).await,
            BrowserCommand::Snapshot => {
                let snapshot = self.collect(session).await?;
                let summary = format!("共 {} 个可交互元素", snapshot.elements.len());
                Ok(CommandOutcome {
                    summary,
                    snapshot: Some(snapshot),
                    ..Default::default()
                })
            }
            BrowserCommand::Click { r#ref } => self.click(session, &r#ref).await,
            BrowserCommand::Type {
                r#ref,
                text,
                submit,
            } => self.type_text(session, &r#ref, &text, submit).await,
            BrowserCommand::Press { key } => self.press(session, &key).await,
            BrowserCommand::Screenshot { full_page } => self.screenshot(session, full_page).await,
            BrowserCommand::Evaluate { expression } => self.evaluate(session, &expression).await,
            BrowserCommand::Close => {
                if let Some(mut live) = guard.take() {
                    live.browser.shutdown().await;
                }
                Ok(CommandOutcome {
                    summary: "浏览器已关闭".into(),
                    ..Default::default()
                })
            }
        }
    }

    async fn close(&self) -> AppResult<()> {
        if let Some(mut live) = self.session.lock().await.take() {
            live.browser.shutdown().await;
        }
        Ok(())
    }
}

/// 连接默认页面（about:blank）的调试 WebSocket。
async fn attach_page(browser: &LaunchedBrowser) -> AppResult<CdpConnection> {
    let url = format!("http://127.0.0.1:{}/json/list", browser.debug_port);
    let response = tokio::time::timeout(Duration::from_secs(10), reqwest::get(&url))
        .await
        .map_err(|_| AppError::new("CDP_TARGET", "获取调试目标超时"))?
        .map_err(|e| AppError::new("CDP_TARGET", format!("获取调试目标失败：{e}")))?;
    let targets: Vec<Value> = tokio::time::timeout(Duration::from_secs(10), response.json())
        .await
        .map_err(|_| AppError::new("CDP_TARGET", "解析调试目标超时"))?
        .map_err(|e| AppError::new("CDP_TARGET", format!("解析调试目标失败：{e}")))?;
    let ws = targets
        .iter()
        .find(|t| t["type"] == "page")
        .and_then(|t| t["webSocketDebuggerUrl"].as_str())
        .ok_or_else(|| AppError::new("CDP_TARGET", "未找到可用的页面调试目标"))?;
    CdpConnection::connect(ws).await
}

async fn page_info(session: &LiveSession) -> AppResult<(String, String)> {
    let value = session
        .cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": "JSON.stringify({url: location.href, title: document.title})",
                "returnByValue": true
            }),
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

async fn dispatch_click(cdp: &CdpConnection, (x, y): (f64, f64)) -> AppResult<()> {
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1}),
    )
    .await?;
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1}),
    )
    .await?;
    Ok(())
}

async fn press_key(cdp: &CdpConnection, key: &str) -> AppResult<()> {
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
    cdp.call("Input.dispatchKeyEvent", params.clone()).await?;
    params["type"] = json!("keyUp");
    cdp.call("Input.dispatchKeyEvent", params).await?;
    Ok(())
}

fn translate_key(key: &str) -> Option<(String, String, u32)> {
    if key.len() == 1 {
        let lower = key.chars().next()?.to_ascii_lowercase();
        if lower.is_ascii_lowercase() {
            let vk = u32::from(lower) - u32::from('a') + 65;
            let upper = lower.to_ascii_uppercase();
            return Some((lower.to_string(), format!("Key{upper}"), vk));
        }
    }
    let named = match key {
        "Enter" => ("Enter", "Enter", 13),
        "Tab" => ("Tab", "Tab", 9),
        "Escape" => ("Escape", "Escape", 27),
        "Backspace" => ("Backspace", "Backspace", 8),
        "Delete" => ("Delete", "Delete", 46),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39),
        "Home" => ("Home", "Home", 36),
        "End" => ("End", "End", 35),
        "PageUp" => ("PageUp", "PageUp", 33),
        "PageDown" => ("PageDown", "PageDown", 34),
        " " | "Space" => (" ", "Space", 32),
        _ => return None,
    };
    Some((named.0.into(), named.1.into(), named.2))
}

fn center(target: Located) -> (f64, f64) {
    (target.x + target.w / 2.0, target.y + target.h / 2.0)
}

/// 解析 PNG IHDR 宽高（固定偏移 16..24，大端 u32）。
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[0..8] != *b"\x89PNG\r\n\x1a\n" || bytes[12..16] != *b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn describe_exception(exception: &Value) -> String {
    exception["exception"]["description"]
        .as_str()
        .or_else(|| exception["text"].as_str())
        .unwrap_or("页面表达式执行失败")
        .to_string()
}
