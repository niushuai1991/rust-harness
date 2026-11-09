//! 内置浏览器能力：向 Runtime 注册 browser_* 工具组（导航、快照、点击、输入、按键、截图、评估、关闭）。
//!
//! 默认后端为系统 Edge/Chrome headless + 轻量 CDP 客户端；命令契约见 [`command`]，
//! 后端抽象见 [`backend::BrowserBackend`]，测试可通过 [`install_with`] 注入假后端。
pub mod backend;
pub mod cdp;
pub mod command;
pub mod launcher;
pub mod snapshot;

use async_trait::async_trait;
use backend::{BrowserBackend, CdpBackend};
use command::{BrowserCommand, CommandOutcome};
use futures::future::BoxFuture;
use harness_contracts::{AppError, AppResult, ToolDefinition, ToolResult};
use harness_runtime::{ExternalTool, Registration, Runtime, ToolContext};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const TOOL_NAMES: [&str; 8] = [
    "browser_navigate",
    "browser_snapshot",
    "browser_click",
    "browser_type",
    "browser_press",
    "browser_screenshot",
    "browser_evaluate",
    "browser_close",
];
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// 后端工厂：默认实现按需启动 CDP 后端；测试可注入假后端。
pub trait BackendFactory: Send + Sync {
    fn create(&self) -> BoxFuture<'_, AppResult<Arc<dyn BrowserBackend>>>;
}

struct CdpFactory {
    screenshots_dir: PathBuf,
}

impl BackendFactory for CdpFactory {
    fn create(&self) -> BoxFuture<'_, AppResult<Arc<dyn BrowserBackend>>> {
        let dir = self.screenshots_dir.clone();
        Box::pin(async move { Ok(Arc::new(CdpBackend::new(dir)) as Arc<dyn BrowserBackend>) })
    }
}

struct ManagerState {
    /// 懒启动的共享后端；持锁执行保证浏览器命令串行。
    slot: tokio::sync::Mutex<Option<Arc<dyn BrowserBackend>>>,
    factory: Box<dyn BackendFactory>,
    idle_at: Mutex<Option<Instant>>,
    stop: CancellationToken,
}

impl ManagerState {
    async fn run(
        &self,
        command: BrowserCommand,
        cancel: CancellationToken,
    ) -> AppResult<CommandOutcome> {
        let mut slot = self.slot.lock().await;
        if slot.is_none() {
            *slot = Some(self.factory.create().await?);
        }
        let backend = slot.as_ref().expect("backend ensured");
        *self.idle_at.lock().unwrap() = Some(Instant::now());
        let work = backend.execute(command);
        tokio::pin!(work);
        let outcome = tokio::select! {
            _ = cancel.cancelled() => return Err(AppError::new("CANCELLED", "浏览器操作已取消")),
            result = &mut work => result,
        };
        *self.idle_at.lock().unwrap() = Some(Instant::now());
        outcome
    }
}

/// install 返回的所有权句柄；需存活至应用结束（Drop 时注销工具并回收浏览器进程）。
pub struct Installed {
    _registrations: Vec<Registration>,
    state: Arc<ManagerState>,
}

impl Drop for Installed {
    fn drop(&mut self) {
        self.state.stop.cancel();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let state = self.state.clone();
            handle.spawn(async move {
                if let Some(backend) = state.slot.lock().await.take() {
                    let _ = backend.close().await;
                }
            });
        }
        // 无 tokio 上下文时由 kill_on_drop 兜底终止浏览器进程。
    }
}

/// 在 Runtime 上注册 browser_* 工具组；截图保存到 `<数据目录>/browser/screenshots`。
pub fn install(runtime: Arc<Runtime>) -> AppResult<Installed> {
    let dir = runtime
        .workspaces
        .data_dir()
        .join("browser")
        .join("screenshots");
    install_with(
        runtime,
        CdpFactory {
            screenshots_dir: dir,
        },
    )
}

/// 测试入口：注入自定义后端工厂。
pub fn install_with(
    runtime: Arc<Runtime>,
    factory: impl BackendFactory + 'static,
) -> AppResult<Installed> {
    let state = Arc::new(ManagerState {
        slot: tokio::sync::Mutex::new(None),
        factory: Box::new(factory),
        idle_at: Mutex::new(None),
        stop: CancellationToken::new(),
    });
    let mut registrations = Vec::new();
    for name in TOOL_NAMES {
        registrations.push(runtime.register_tool(Arc::new(Tool {
            state: state.clone(),
            name: name.into(),
        }))?);
    }
    spawn_idle_reaper(state.clone());
    Ok(Installed {
        _registrations: registrations,
        state,
    })
}

/// 空闲回收：超过阈值未使用则关闭浏览器进程，下次命令重新懒启动。
fn spawn_idle_reaper(state: Arc<ManagerState>) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = state.stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
            let expired = state
                .idle_at
                .lock()
                .unwrap()
                .is_some_and(|at| at.elapsed() >= IDLE_TIMEOUT);
            if expired && let Some(backend) = state.slot.lock().await.take() {
                let _ = backend.close().await;
                *state.idle_at.lock().unwrap() = None;
                tracing::info!("浏览器空闲超时，已自动关闭");
            }
        }
    });
}

struct Tool {
    state: Arc<ManagerState>,
    name: String,
}

fn definition(name: &str) -> ToolDefinition {
    let text = json!({"type": "string", "minLength": 1});
    let (properties, required, description) = match name {
        "browser_navigate" => (
            json!({"url": text, "action": {"type": "string", "enum": ["back", "reload"]}}),
            vec![],
            "在内置浏览器中打开 URL（仅 http/https/about:blank）；不传 url 且 action=back/reload 时后退或刷新当前页。导航后请先 browser_snapshot 再操作元素",
        ),
        "browser_snapshot" => (
            json!({}),
            vec![],
            "返回当前页面可交互元素清单（ref/role/tag/name/value）；点击或输入前必须先获取快照",
        ),
        "browser_click" => (
            json!({"ref": text}),
            vec!["ref"],
            "点击快照 ref 对应的元素（真实鼠标事件），返回点击后的新快照",
        ),
        "browser_type" => (
            json!({"ref": text, "text": {"type": "string"}, "submit": {"type": "boolean"}}),
            vec!["ref", "text"],
            "向快照 ref 对应的输入元素输入文本；submit 为 true 时输入后按回车",
        ),
        "browser_press" => (
            json!({"key": text}),
            vec!["key"],
            "发送按键（Enter/Tab/Escape/Backspace/Delete/ArrowUp/ArrowDown/ArrowLeft/ArrowRight/Home/End/PageUp/PageDown）",
        ),
        "browser_screenshot" => (
            json!({"fullPage": {"type": "boolean"}}),
            vec![],
            "截取当前页面 PNG 并保存到数据目录，返回文件路径与像素尺寸",
        ),
        "browser_evaluate" => (
            json!({"expression": text}),
            vec!["expression"],
            "在当前页面执行只读 JavaScript 表达式并返回 JSON 结果（读取页面状态用）",
        ),
        "browser_close" => (
            json!({}),
            vec![],
            "关闭浏览器会话并终止 headless 浏览器进程",
        ),
        other => unreachable!("未注册的浏览器工具 {other}"),
    };
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type": "object", "properties": properties, "required": required}),
    }
}

fn build_command(name: &str, args: &Value) -> AppResult<BrowserCommand> {
    let text = |key: &str| -> AppResult<String> {
        args[key]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", format!("缺少有效字段 {key}")))
    };
    Ok(match name {
        "browser_navigate" => match args["action"].as_str() {
            Some("back") => BrowserCommand::Back,
            Some("reload") => BrowserCommand::Reload,
            Some(other) => {
                return Err(AppError::new(
                    "TOOL_ARGUMENTS",
                    format!("无效 action：{other}"),
                ));
            }
            None => BrowserCommand::Navigate { url: text("url")? },
        },
        "browser_snapshot" => BrowserCommand::Snapshot,
        "browser_click" => BrowserCommand::Click {
            r#ref: text("ref")?,
        },
        "browser_type" => BrowserCommand::Type {
            r#ref: text("ref")?,
            text: text("text")?,
            submit: args["submit"].as_bool().unwrap_or(false),
        },
        "browser_press" => BrowserCommand::Press { key: text("key")? },
        "browser_screenshot" => BrowserCommand::Screenshot {
            full_page: args["fullPage"].as_bool().unwrap_or(false),
        },
        "browser_evaluate" => BrowserCommand::Evaluate {
            expression: text("expression")?,
        },
        "browser_close" => BrowserCommand::Close,
        other => {
            return Err(AppError::new(
                "TOOL_ARGUMENTS",
                format!("未知浏览器工具 {other}"),
            ));
        }
    })
}

#[async_trait]
impl ExternalTool for Tool {
    fn definition(&self) -> ToolDefinition {
        definition(&self.name)
    }

    async fn execute(
        &self,
        _context: ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        let command = build_command(&self.name, &args)?;
        let outcome = self.state.run(command, cancel).await?;
        Ok(ToolResult {
            content: serde_json::to_string(&outcome)
                .map_err(|e| AppError::new("INTERNAL", e.to_string()))?,
            is_error: false,
            escalated: false,
        })
    }
}
