//! 浏览器命令契约：所有后端共享的唯一命令面（对齐 ZCode BrowserCommand 的分层思路）。
use serde::{Deserialize, Serialize};

/// 后端唯一认识的命令集合；新增后端（如 WebView2 可视后端）只需实现同一契约。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserCommand {
    /// 打开 URL；scheme 白名单在执行层校验。
    Navigate { url: String },
    /// 后退一页。
    Back,
    /// 刷新当前页。
    Reload,
    /// 收集当前页面可交互元素清单。
    Snapshot,
    /// 点击快照中 ref 对应的元素。
    Click {
        #[serde(rename = "ref")]
        r#ref: String,
    },
    /// 向 ref 对应元素输入文本，`submit` 为 true 时追加回车。
    Type {
        #[serde(rename = "ref")]
        r#ref: String,
        text: String,
        submit: bool,
    },
    /// 发送单个按键。
    Press { key: String },
    /// 截图并保存 PNG。
    Screenshot { full_page: bool },
    /// 在页面执行 JavaScript 表达式并返回结果。
    Evaluate { expression: String },
    /// 关闭浏览器会话。
    Close,
}

/// 命令执行结果；快照为 `None` 时模型可另行调用 browser_snapshot。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutcome {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ScreenshotInfo>,
}

/// 可交互元素清单视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotView {
    pub url: String,
    pub title: String,
    pub elements: Vec<ElementInfo>,
}

/// 单个可交互元素；`ref` 是 Rust 侧映射表的键，页面脚本不可触碰。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub role: String,
    pub tag: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// 截图产物信息；ToolResult 为纯文本模型，PNG 落盘后仅回传路径与尺寸。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

/// 导航白名单：仅 http/https 与 about:blank，拒绝 file/data/javascript 等其余 scheme。
pub fn is_allowed_url(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if !rest.starts_with("//") || rest.len() <= 2 {
        return false;
    }
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
}
