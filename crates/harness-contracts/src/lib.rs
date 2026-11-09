//! Shared wire vocabulary. Persistent event cursors are decimal strings to avoid JS precision loss.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
/// A named recursive JSON definition prevents infinite inlining during TS export.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    Number(f64),
    Text(String),
    Array(Vec<JsonValue>),
    Object(std::collections::HashMap<String, JsonValue>),
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct JsonPayload(#[specta(type = JsonValue)] pub Value);

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}
impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AppError {}
pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: String,
    pub source_path: String,
    pub replica_path: String,
    pub created_at: String,
    /// 项目显示名；创建未填时归一化为源目录名，加载旧记录（无此字段）时空值同样归一化，永不为空。
    #[serde(default)]
    pub name: String,
    /// 会话列表「项目」视图置顶标记；工作区记录本身不落库，由桌面端从 settings KV 派生注入。
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub kind: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub source_hash: Option<String>,
    pub conflict: bool,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

/// 单文件 unified diff 视图（桌面端右侧面板懒加载）；unified 为 None 表示二进制或超限。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileDiff {
    pub path: String,
    pub unified: Option<String>,
}

/// 工作副本文件的有界预览（桌面端右侧面板）。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePreview {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub binary: bool,
    #[specta(type = f64)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    /// 子代理归属的父会话；由运行时从 subagent.created 事件推导，存储层恒为空。
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub seq: String,
    pub kind: String,
    #[specta(type = JsonValue)]
    pub data: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LiveEvent {
    Durable {
        event: SessionEvent,
    },
    Delta {
        session_id: String,
        request_id: String,
        index: u32,
        text: String,
        reasoning: bool,
    },
    Status {
        session_id: String,
        status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[specta(type = JsonValue)]
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[specta(type = JsonValue)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    #[specta(type = JsonValue)]
    pub arguments: Value,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ApiMode {
    #[default]
    ChatCompletions,
    Responses,
    Anthropic,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEndpoint {
    pub api_mode: ApiMode,
    pub base_url: String,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProviderCompatibility {
    #[default]
    Standard,
    DeepSeek,
    BigModel,
}

/// 供应商认证方式：静态 API key 或 ChatGPT 账号 OAuth（token 由登录流程动态维护）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAuth {
    #[default]
    ApiKey,
    ChatGptOAuth,
}
/// 已知推理档位，语义强度从低到高；efforts 为空表示该模型不提供 effort 选择。
pub const KNOWN_EFFORTS: [&str; 6] = ["disabled", "minimal", "low", "medium", "high", "max"];

/// 已知审批策略档位（五档权限模式），与 CLI `--approval`、ACP modes、桌面选择器一一对应。
pub const KNOWN_APPROVALS: [&str; 5] = ["ask", "edit", "auto", "never", "guarded"];

/// 审批策略档位：一次审批请求由谁裁决。
///
/// `decide` 返回 `Some(allow)` 表示按档位直接结算；返回 `None` 表示该档位无法自动裁决，
/// 交由宿主（终端 y/n、桌面审批卡片、ACP `session/request_permission`）交互决定。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalPolicy {
    /// 每次确认：一律交互询问。
    #[default]
    Ask,
    /// 自动编辑：`write`/`edit` 自动放行，其余工具仍询问。
    Edit,
    /// 完全访问：全部自动放行（改动仍只落工作副本，进原项目仍需 apply）。
    Auto,
    /// 只读：需审批工具一律自动拒绝。
    Never,
    /// 自主：全部自动放行，但 `shell` 命中[`dangerous_shell_rule`]时询问。
    Guarded,
}

impl ApprovalPolicy {
    /// 解析档位字符串（[`KNOWN_APPROVALS`] 内的值，大小写不敏感）。
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.to_ascii_lowercase().as_str() {
            "ask" => Self::Ask,
            "edit" => Self::Edit,
            "auto" => Self::Auto,
            "never" => Self::Never,
            "guarded" => Self::Guarded,
            _ => return None,
        })
    }

    /// 档位的规范字符串形式（即 [`KNOWN_APPROVALS`] 中的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Edit => "edit",
            Self::Auto => "auto",
            Self::Never => "never",
            Self::Guarded => "guarded",
        }
    }

    /// 单次审批裁决：各宿主共用的唯一决策实现。
    pub fn decide(self, request: &ApprovalRequest) -> Option<bool> {
        match self {
            Self::Ask => None,
            Self::Auto => Some(true),
            Self::Never => Some(false),
            Self::Edit => matches!(request.tool.as_str(), "write" | "edit").then_some(true),
            Self::Guarded => {
                if request.tool != "shell" {
                    return Some(true);
                }
                let command = request.arguments.get("command").and_then(Value::as_str)?;
                if command.trim().is_empty() || dangerous_shell_rule(command).is_some() {
                    None
                } else {
                    Some(true)
                }
            }
        }
    }
}

/// 自主（guarded）档的危险 shell 命令识别：命中返回规则标识，未命中返回 `None`。
///
/// 纯文本启发式，不解析完整 shell 语法：
/// - 按 `&&`/`||`/`;`/`&`/`|`/换行分段逐段评估，支持一层常见包装命令（`cmd /c`、`powershell -c` 等）；
/// - 含动态展开（`%VAR%`、`$var`/`$()`、反引号）或超长命令一律保守视为需确认（`unsupported`）；
/// - 含 `--help`/`-h`/`/?` 的查询命令不拦；
/// - 覆盖破坏性 git 操作、递归删除与系统级命令（Windows 优先，兼容 POSIX 写法）。
pub fn dangerous_shell_rule(command: &str) -> Option<&'static str> {
    if command.len() > 4096 {
        return Some("unsupported");
    }
    split_segments(command)
        .iter()
        .find_map(|segment| evaluate_segment(segment, 0))
}

/// 引号感知的分段：分段符在引号外才生效，连续分段符合并。
fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    current.push(c);
                }
                ';' | '&' | '|' | '\n' => {
                    while matches!(chars.peek(), Some('&' | '|' | ';')) {
                        chars.next();
                    }
                    segments.push(std::mem::take(&mut current));
                }
                _ => current.push(c),
            },
        }
    }
    segments.push(current);
    segments
}

/// 递归深度上限：包装命令最多剥三层。
const MAX_UNWRAP_DEPTH: u8 = 3;

fn evaluate_segment(segment: &str, depth: u8) -> Option<&'static str> {
    if depth >= MAX_UNWRAP_DEPTH {
        return Some("unsupported");
    }
    let tokens = tokenize(segment);
    if tokens.is_empty() {
        return None;
    }
    // 查询命令不拦。
    if tokens
        .iter()
        .any(|t| t == "-h" || t == "--help" || t == "/?")
    {
        return None;
    }
    // 引号外的动态展开无法静态求值，保守询问。
    if has_dynamic_expansion(segment) {
        return Some("unsupported");
    }
    // 包装命令（cmd /c …、powershell -command …、bash -c …）剥壳后评估内层。
    if let Some(inner) = unwrap_wrapper(&tokens) {
        return evaluate_segment(&inner, depth + 1);
    }
    rule_for(&tokens)
}

/// 引号感知分词：返回去引号后的 token 列表。
fn tokenize(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;
    for c in segment.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    has_token = true;
                }
                c if c.is_whitespace() => {
                    if has_token {
                        tokens.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                }
                _ => {
                    current.push(c);
                    has_token = true;
                }
            },
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

/// 引号外（单引号内除外）出现变量或子命令展开记为动态片段。
fn has_dynamic_expansion(segment: &str) -> bool {
    let mut quote: Option<char> = None;
    for c in segment.chars() {
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
            }
            Some(_) => {
                if c == '"' {
                    quote = None;
                }
            }
            None => match c {
                '\'' => quote = Some('\''),
                '"' => quote = Some('"'),
                '%' | '$' | '`' => return true,
                _ => {}
            },
        }
    }
    false
}

/// 已知包装命令：返回内层命令串（后续 token 重新拼接）。
fn unwrap_wrapper(tokens: &[String]) -> Option<String> {
    let program = tokens[0].rsplit(['/', '\\']).next().unwrap_or(&tokens[0]);
    let program = program.strip_suffix(".exe").unwrap_or(program);
    let is_wrapper = matches!(
        program.to_ascii_lowercase().as_str(),
        "cmd" | "powershell" | "pwsh" | "bash" | "sh" | "wsl" | "start"
    );
    if !is_wrapper {
        return None;
    }
    // 跳过包装参数（/c、/k、-c、-command、/d /s 等），内层命令从首个非 flag token 开始。
    let mut index = 1;
    while index < tokens.len() && tokens[index].starts_with('-')
        || (index < tokens.len() && tokens[index].starts_with('/'))
    {
        index += 1;
    }
    if index >= tokens.len() {
        return None;
    }
    Some(tokens[index..].join(" "))
}

/// 识别一条命令的规则；`tokens` 为去引号 token。
fn rule_for(tokens: &[String]) -> Option<&'static str> {
    let program = tokens[0].rsplit(['/', '\\']).next().unwrap_or(&tokens[0]);
    let program = program
        .strip_suffix(".exe")
        .or_else(|| program.strip_suffix(".cmd"))
        .or_else(|| program.strip_suffix(".bat"))
        .or_else(|| program.strip_suffix(".com"))
        .or_else(|| program.strip_suffix(".ps1"))
        .unwrap_or(program)
        .to_ascii_lowercase();
    let args = &tokens[1..];
    match program.as_str() {
        "git" => git_rule(args),
        "rm" => rm_rule(args),
        "rd" | "rmdir" => rd_rule(args),
        "del" | "erase" => del_rule(args),
        "remove-item" | "ri" => remove_item_rule(args),
        "dd" | "mkfs" | "mkfs.ntfs" | "mkfs.ext2" | "mkfs.ext3" | "mkfs.ext4" | "mkfs.vfat"
        | "mount" | "umount" | "diskpart" | "shutdown" | "format" => Some("system-storage"),
        "cipher"
            if args.iter().any(|t| {
                t.eq_ignore_ascii_case("/w") || t.to_ascii_lowercase().starts_with("/w:")
            }) =>
        {
            Some("system-storage")
        }
        "reg"
            if args
                .first()
                .is_some_and(|t| t.eq_ignore_ascii_case("delete")) =>
        {
            Some("system-storage")
        }
        "robocopy"
            if args
                .iter()
                .any(|t| t.eq_ignore_ascii_case("/mir") || t.eq_ignore_ascii_case("/purge")) =>
        {
            Some("system-storage")
        }
        _ => None,
    }
}

/// git 子命令规则：跳过全局选项（含取值选项）后按子命令判定。
fn git_rule(args: &[String]) -> Option<&'static str> {
    let mut index = 0;
    while index < args.len() {
        let token = args[index].as_str();
        if matches!(
            token,
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "-c"
        ) {
            index += 2;
        } else if token.starts_with('-') {
            index += 1;
        } else {
            break;
        }
    }
    let sub = args.get(index)?;
    let rest = &args[index + 1..];
    match sub.as_str() {
        "reset" if rest.iter().any(|t| t == "--hard") => Some("git-reset-hard"),
        "clean" if rest.iter().any(|t| clean_force_flag(t)) => Some("git-clean-force"),
        "push" if !rest.iter().any(|t| t == "--dry-run") => rest
            .iter()
            .any(|t| {
                t == "-f"
                    || t == "--force"
                    || t == "--force-with-lease"
                    || t == "--mirror"
                    || t.starts_with('+')
            })
            .then_some("git-push-force"),
        "stash" if rest.first().is_some_and(|t| t == "drop" || t == "clear") => {
            Some("git-stash-clear")
        }
        "worktree" if rest.first().is_some_and(|t| t == "remove") => rest
            .iter()
            .any(|t| t == "--force" || t == "-f")
            .then_some("git-worktree-force-remove"),
        "checkout" if rest.iter().any(|t| t == "--" || t == "." || t == "..") => {
            Some("git-discard-tree")
        }
        "restore" => Some("git-discard-tree"),
        _ => None,
    }
}

/// `git clean` 的删除性旗标：`-f`/`--force` 及含 `f` 的短旗标组合（`-fd` 等）。
fn clean_force_flag(token: &str) -> bool {
    token == "--force"
        || (token.starts_with('-') && !token.starts_with("--") && token[1..].contains('f'))
}

/// POSIX `rm`：递归删除且（多操作数或含 glob）判危险；根路径/主目录缩写无论操作数个数一律判危险。
fn rm_rule(args: &[String]) -> Option<&'static str> {
    let recursive = args.iter().any(|t| {
        t == "--recursive"
            || (t.starts_with('-') && !t.starts_with("--") && t[1..].contains(['r', 'R']))
    });
    if !recursive {
        return None;
    }
    let operands: Vec<&String> = args.iter().filter(|t| !t.starts_with('-')).collect();
    if operands.iter().any(|t| is_root_like(t)) {
        return Some("remove-critical-path");
    }
    let glob = operands.iter().any(|t| t.contains(['*', '?', '[']));
    (operands.len() >= 2 || glob).then_some("remove-critical-path")
}

/// 根路径、盘符根或 `~` 主目录缩写。
fn is_root_like(token: &str) -> bool {
    let t = token.trim();
    t == "/"
        || t == "\\"
        || t == "~"
        || t.starts_with("~/")
        || t.starts_with("~\\")
        || t.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && matches!(t.as_bytes().get(1), Some(b':'))
            && t[2..].chars().all(|c| c == '\\' || c == '/')
}

/// cmd `rd`/`rmdir`：`/s` 递归删除。
fn rd_rule(args: &[String]) -> Option<&'static str> {
    args.iter()
        .any(|t| t.eq_ignore_ascii_case("/s"))
        .then_some("remove-critical-path")
}

/// cmd `del`/`erase`：`/s` 递归子目录删除。
fn del_rule(args: &[String]) -> Option<&'static str> {
    args.iter()
        .any(|t| t.eq_ignore_ascii_case("/s"))
        .then_some("remove-critical-path")
}

/// PowerShell `Remove-Item`：`-Recurse`/`-r` 递归删除。
fn remove_item_rule(args: &[String]) -> Option<&'static str> {
    args.iter()
        .any(|t| t.eq_ignore_ascii_case("-recurse") || t.eq_ignore_ascii_case("-r"))
        .then_some("remove-critical-path")
}

/// 内置供应商随程序分发的只读模型条目；启用状态由个人层的 `disabled_models` 覆盖。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinModel {
    pub id: String,
    pub name: String,
    pub efforts: Vec<String>,
    /// 上下文窗口（tokens）；旧存储缺省为 None，未知窗口时前端隐藏用量指示器。
    #[serde(default)]
    pub context_window: Option<u32>,
    /// 最大输出（tokens）；旧存储缺省为 None，未配置时请求按协议方言的兜底值发送。
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    #[serde(default)]
    pub auth: ProviderAuth,
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub endpoints: Vec<ProtocolEndpoint>,
    pub default_model: String,
    pub credential_ref: String,
    pub proxy: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub builtin_models: Vec<BuiltinModel>,
    #[serde(default)]
    pub disabled_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    #[serde(default)]
    pub compatibility: ProviderCompatibility,
    #[serde(default)]
    pub auth: ProviderAuth,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub api_mode: ApiMode,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// 上下文窗口（tokens）；旧存储缺省为 None，未知窗口时前端隐藏用量指示器。
    #[serde(default)]
    pub context_window: Option<u32>,
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub credential_ref: String,
    pub proxy: Option<String>,
    /// 支持的推理档位；为空表示不支持，UI 隐藏 effort 选择。
    #[serde(default)]
    pub efforts: Vec<String>,
    /// 当前默认档位；None 表示不干预、跟随供应商默认行为。
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 由内置目录合成的条目；不可编辑或删除，只能启用/禁用。
    #[serde(default)]
    pub builtin: bool,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            compatibility: ProviderCompatibility::default(),
            auth: ProviderAuth::default(),
            provider_id: String::new(),
            api_mode: ApiMode::default(),
            max_output_tokens: None,
            context_window: None,
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
            credential_ref: String::new(),
            proxy: None,
            efforts: Vec::new(),
            effort: None,
            enabled: true,
            builtin: false,
        }
    }
}

/// 供应商剩余额度的卡片类别；与智谱 quota/limit 接口的 type/unit 匹配规则对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UsageCardKind {
    FiveHour,
    Weekly,
    Tools,
}

/// 单张剩余额度卡；`remaining_percent` 为剩余百分比（接口的已用口径已反转），`reset_at` 为毫秒时间戳。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageCard {
    pub kind: UsageCardKind,
    pub remaining_percent: Option<f64>,
    /// 毫秒时间戳远小于 2^53，specta 以 f64 导出 number，绕开 BigInt 禁令。
    #[specta(type = f64)]
    pub reset_at: Option<i64>,
}

/// 供应商套餐与剩余额度快照；即查即返，不落库。
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageSnapshot {
    /// 套餐显示名（如 GLM Coding Pro）；由接口 level 字段归一化而来。
    pub plan_name: Option<String>,
    pub cards: Vec<ProviderUsageCard>,
}

/// ChatGPT 登录流程状态机（桌面端轮询）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChatGptLoginStatus {
    /// 本窗口未发起过登录。
    #[default]
    Idle,
    /// 等待浏览器回调。
    Pending,
    /// 登录成功且凭据已保存。
    Completed,
    /// 登录失败（error 携带原因）。
    Failed,
    /// 用户取消。
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptLoginState {
    pub status: ChatGptLoginStatus,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub error: Option<String>,
}

/// ChatGPT 账号登录信息查询；不含任何 token。
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAccountInfo {
    pub logged_in: bool,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    /// access token 到期时间（Unix 秒，远小于 2^53）。
    #[specta(type = f64)]
    pub expires_at: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// 桌面端跨重启保留的上次模型选择；effort 为 None 表示跟随模型默认。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSelection {
    pub model_id: String,
    pub effort: Option<String>,
}

/// 用户自定义预设：命名的会话能力配置包（模型/effort/审批策略），可选字段缺省表示该项不覆盖。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresetConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub approval: Option<String>,
}

/// 预设列表行：配置附运行时计算的 broken 标记（不落库），可读但不可直接构建模型。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    #[serde(flatten)]
    pub preset: PresetConfig,
    /// 预设不可用的原因（模型缺失/停用、effort 不支持等）；None 表示可用。
    pub broken: Option<String>,
}

/// 技能目录行：SKILL.md 扫描结果附禁用标记（docs/specs/skills.md）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// 项目级为相对工作副本的路径，用户级为绝对路径；统一正斜杠。
    pub path: String,
    pub scope: SkillScope,
    pub enabled: bool,
    /// 技能不可用的原因（name 非法等）；None 表示可用。
    #[serde(default)]
    pub broken_reason: Option<String>,
}

/// 技能来源层：项目级（工作副本）与用户级（主目录 `~/.agents/skills/`）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillScope {
    Project,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub enabled: bool,
    pub permissions: Vec<String>,
    pub status: String,
    pub description: String,
}

/// Host-managed, content-addressed attachment metadata. Never includes the imported host path.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub hash: String,
    pub name: String,
    pub media_type: String,
    pub size: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChunk {
    pub encoding: String,
    pub content: String,
    pub next_offset: Option<u32>,
    pub total_size: u32,
}

/// One configured language server: the command runs inside the platform sandbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LspServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Server-specific `initializationOptions`; the harness pushes file contents
    /// itself, so servers needing it may disable their internal file watchers here.
    #[serde(default)]
    #[specta(type = JsonValue)]
    pub initialization_options: Value,
}

impl LspServerConfig {
    pub fn default_servers() -> Vec<Self> {
        vec![Self {
            id: "rust-analyzer".into(),
            command: "rust-analyzer".into(),
            args: vec![],
            extensions: vec!["rs".into()],
            initialization_options: Value::Null,
        }]
    }
}

/// Runtime state of one language server connection, for host status queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LspStatusEntry {
    pub workspace_id: String,
    pub server_id: String,
    pub running: bool,
    pub documents: u32,
    pub diagnostics: u32,
}

/// Latest todo/plan/goal entry of one session for the desktop overview panel.
/// Derived view over the event log; the event stream stays the source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanStateOverview {
    pub session_id: String,
    pub session_title: String,
    pub workspace_id: String,
    pub kind: String,
    #[specta(type = JsonValue)]
    pub data: Value,
    pub updated_at: String,
}

#[cfg(test)]
mod lsp_config_tests {
    use super::*;

    #[test]
    fn default_servers_map_rust_to_rust_analyzer() {
        let servers = LspServerConfig::default_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "rust-analyzer");
        assert_eq!(servers[0].command, "rust-analyzer");
        assert_eq!(servers[0].extensions, vec!["rs"]);
        assert!(servers[0].args.is_empty());
    }

    #[test]
    fn config_roundtrips_with_missing_optional_fields() {
        let config: LspServerConfig = serde_json::from_str(
            r#"{"id":"pyright","command":"pyright-langserver","extensions":["py"]}"#,
        )
        .unwrap();
        assert_eq!(config.args, Vec::<String>::new());
        assert_eq!(config.extensions, vec!["py"]);
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    /// 预设可选字段序列化缺省；camelCase 键名与桌面绑定一致。
    #[test]
    fn preset_roundtrips_with_optional_fields() {
        let preset = PresetConfig {
            id: "refactor".into(),
            name: "批量重构".into(),
            description: Some("自动审批跑批量任务".into()),
            model_id: Some("builtin:bigmodel:glm-5.3".into()),
            effort: Some("high".into()),
            approval: Some("auto".into()),
        };
        let json = serde_json::to_string(&preset).unwrap();
        assert!(json.contains("\"modelId\""));
        let back: PresetConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "refactor");
        assert_eq!(back.model_id.as_deref(), Some("builtin:bigmodel:glm-5.3"));
        assert_eq!(back.effort.as_deref(), Some("high"));
        assert_eq!(back.approval.as_deref(), Some("auto"));
    }

    /// 旧存储/手写 JSON 允许只给 id，其余字段全部缺省。
    #[test]
    fn preset_minimal_json_defaults() {
        let preset: PresetConfig = serde_json::from_str(r#"{"id":"p1","name":"p1"}"#).unwrap();
        assert_eq!(preset.description, None);
        assert_eq!(preset.model_id, None);
        assert_eq!(preset.effort, None);
        assert_eq!(preset.approval, None);
    }

    /// 审批档位集合与五档权限模式一一对应。
    #[test]
    fn known_approvals_cover_policy_set() {
        assert_eq!(KNOWN_APPROVALS, ["ask", "edit", "auto", "never", "guarded"]);
    }
}

#[cfg(test)]
mod approval_policy_tests {
    use super::*;

    fn request(tool: &str, arguments: Value) -> ApprovalRequest {
        ApprovalRequest {
            id: "a1".into(),
            session_id: "s1".into(),
            tool: tool.into(),
            arguments,
            reason: "测试".into(),
        }
    }

    fn shell(command: &str) -> ApprovalRequest {
        request("shell", serde_json::json!({"command": command}))
    }

    #[test]
    fn parse_and_serde_roundtrip_all_policies() {
        for value in KNOWN_APPROVALS {
            let policy = ApprovalPolicy::parse(value).unwrap();
            assert_eq!(policy.as_str(), value);
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(json, format!("\"{value}\""));
            let back: ApprovalPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(back, policy);
        }
        assert_eq!(ApprovalPolicy::parse("yolo"), None);
        assert_eq!(ApprovalPolicy::parse("ASK"), Some(ApprovalPolicy::Ask));
    }

    #[test]
    fn ask_auto_never_decide_uniformly() {
        let shell_req = shell("ls");
        let write_req = request("write", serde_json::json!({"path": "a.txt"}));
        let mcp_req = request("mcp__srv__tool", serde_json::json!({}));
        for req in [&shell_req, &write_req, &mcp_req] {
            assert_eq!(ApprovalPolicy::Ask.decide(req), None, "ask 恒交互");
            assert_eq!(ApprovalPolicy::Auto.decide(req), Some(true), "auto 恒放行");
            assert_eq!(
                ApprovalPolicy::Never.decide(req),
                Some(false),
                "never 恒拒绝"
            );
        }
    }

    #[test]
    fn edit_allows_file_tools_only() {
        let write_req = request("write", serde_json::json!({"path": "a.txt"}));
        let edit_req = request("edit", serde_json::json!({"path": "a.txt"}));
        let shell_req = shell("npm test");
        let mcp_req = request("mcp__srv__tool", serde_json::json!({}));
        assert_eq!(ApprovalPolicy::Edit.decide(&write_req), Some(true));
        assert_eq!(ApprovalPolicy::Edit.decide(&edit_req), Some(true));
        assert_eq!(ApprovalPolicy::Edit.decide(&shell_req), None);
        assert_eq!(ApprovalPolicy::Edit.decide(&mcp_req), None);
    }

    #[test]
    fn guarded_allows_safe_tools_and_asks_on_danger() {
        assert_eq!(
            ApprovalPolicy::Guarded.decide(&request("write", serde_json::json!({}))),
            Some(true)
        );
        assert_eq!(
            ApprovalPolicy::Guarded.decide(&shell("cargo test")),
            Some(true)
        );
        assert_eq!(
            ApprovalPolicy::Guarded.decide(&shell("git push --force")),
            None
        );
        // 命令缺失或为空时保守询问。
        assert_eq!(
            ApprovalPolicy::Guarded.decide(&request("shell", serde_json::json!({}))),
            None
        );
        assert_eq!(ApprovalPolicy::Guarded.decide(&shell("   ")), None);
    }

    #[test]
    fn dangerous_git_operations_match_rules() {
        assert_eq!(
            dangerous_shell_rule("git reset --hard"),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("git clean -fd"),
            Some("git-clean-force")
        );
        assert_eq!(dangerous_shell_rule("git clean -n"), None);
        assert_eq!(
            dangerous_shell_rule("git push --force origin main"),
            Some("git-push-force")
        );
        assert_eq!(
            dangerous_shell_rule("git push --force-with-lease"),
            Some("git-push-force")
        );
        assert_eq!(
            dangerous_shell_rule("git push origin +main"),
            Some("git-push-force")
        );
        assert_eq!(dangerous_shell_rule("git push --dry-run origin main"), None);
        assert_eq!(dangerous_shell_rule("git push -h"), None);
        assert_eq!(
            dangerous_shell_rule("git stash clear"),
            Some("git-stash-clear")
        );
        assert_eq!(
            dangerous_shell_rule("git stash drop"),
            Some("git-stash-clear")
        );
        assert_eq!(dangerous_shell_rule("git stash"), None);
        assert_eq!(
            dangerous_shell_rule("git worktree remove --force"),
            Some("git-worktree-force-remove")
        );
        assert_eq!(
            dangerous_shell_rule("git checkout -- ."),
            Some("git-discard-tree")
        );
        assert_eq!(dangerous_shell_rule("git checkout main"), None);
        assert_eq!(
            dangerous_shell_rule("git restore src/lib.rs"),
            Some("git-discard-tree")
        );
        // 全局选项不干扰子命令识别。
        assert_eq!(
            dangerous_shell_rule("git -C repo reset --hard"),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("git -c a=b push -f"),
            Some("git-push-force")
        );
        // 引号内文本不误判。
        assert_eq!(
            dangerous_shell_rule("git commit -m \"fix --hard bug\""),
            None
        );
    }

    #[test]
    fn dangerous_deletion_commands_match_rules() {
        assert_eq!(
            dangerous_shell_rule("rm -rf build dist"),
            Some("remove-critical-path")
        );
        assert_eq!(
            dangerous_shell_rule("rm -r *.tmp"),
            Some("remove-critical-path")
        );
        assert_eq!(
            dangerous_shell_rule("rm -rf /"),
            Some("remove-critical-path")
        );
        assert_eq!(
            dangerous_shell_rule("rm -rf ~"),
            Some("remove-critical-path")
        );
        assert_eq!(
            dangerous_shell_rule("rm -rf C:\\"),
            Some("remove-critical-path")
        );
        assert_eq!(dangerous_shell_rule("rm file.txt"), None);
        assert_eq!(
            dangerous_shell_rule("rm -rf build"),
            None,
            "单字面操作数不拦（zcode 语义）"
        );
        assert_eq!(
            dangerous_shell_rule("rd /s /q temp"),
            Some("remove-critical-path")
        );
        assert_eq!(
            dangerous_shell_rule("RD /S /Q temp"),
            Some("remove-critical-path")
        );
        assert_eq!(dangerous_shell_rule("rmdir temp"), None);
        assert_eq!(
            dangerous_shell_rule("del /f /s *.log"),
            Some("remove-critical-path")
        );
        assert_eq!(dangerous_shell_rule("del temp.txt"), None);
        assert_eq!(
            dangerous_shell_rule("Remove-Item -Recurse -Force dist"),
            Some("remove-critical-path")
        );
        assert_eq!(dangerous_shell_rule("Remove-Item dist"), None);
    }

    #[test]
    fn dangerous_system_commands_match_rules() {
        assert_eq!(
            dangerous_shell_rule("dd if=/dev/zero of=disk"),
            Some("system-storage")
        );
        assert_eq!(dangerous_shell_rule("shutdown /r"), Some("system-storage"));
        assert_eq!(dangerous_shell_rule("format D:"), Some("system-storage"));
        assert_eq!(
            dangerous_shell_rule("robocopy a b /mir"),
            Some("system-storage")
        );
        assert_eq!(dangerous_shell_rule("robocopy a b"), None);
        assert_eq!(
            dangerous_shell_rule("reg delete HKLM\\\\X"),
            Some("system-storage")
        );
        assert_eq!(dangerous_shell_rule("reg query HKLM\\\\X"), None);
        assert_eq!(dangerous_shell_rule("cipher /w:C"), Some("system-storage"));
        // 路径形式不干扰识别；`-h` 豁免覆盖 `shutdown -h`（zcode 同语义，已知局限）。
        assert_eq!(
            dangerous_shell_rule("/sbin/shutdown /r"),
            Some("system-storage")
        );
    }

    #[test]
    fn segments_chains_and_wrappers_are_evaluated() {
        assert_eq!(
            dangerous_shell_rule("git add . && git reset --hard"),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("git add .; git push --force"),
            Some("git-push-force")
        );
        assert_eq!(
            dangerous_shell_rule("echo hi | git reset --hard"),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("cmd /c \"git reset --hard\""),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("cmd /c git reset --hard"),
            Some("git-reset-hard")
        );
        assert_eq!(
            dangerous_shell_rule("powershell -Command \"git push --force\""),
            Some("git-push-force")
        );
        assert_eq!(dangerous_shell_rule("cmd /c \"cargo test\""), None);
    }

    #[test]
    fn dynamic_fragments_and_help_queries_are_conservative() {
        assert_eq!(
            dangerous_shell_rule("git push $BRANCH"),
            Some("unsupported")
        );
        assert_eq!(
            dangerous_shell_rule("git push origin %BRANCH%"),
            Some("unsupported")
        );
        assert_eq!(dangerous_shell_rule("del `ls`"), Some("unsupported"));
        // 单引号内的文本不视为动态展开。
        assert_eq!(dangerous_shell_rule("grep '$foo' file"), None);
        assert_eq!(dangerous_shell_rule("rm --help"), None);
        assert_eq!(dangerous_shell_rule("git reset --hard --help"), None);
        assert_eq!(dangerous_shell_rule("shutdown /?"), None);
        let long = format!("git push --force {}", "a".repeat(5000));
        assert_eq!(dangerous_shell_rule(&long), Some("unsupported"));
    }
}

#[cfg(test)]
mod model_settings_tests {
    use super::*;

    /// 升级兼容：v2 存储中的旧供应商/模型 JSON 不含新增字段，必须原样读入并取默认值。
    #[test]
    fn legacy_provider_and_model_json_defaults() {
        let provider: ProviderInfo = serde_json::from_str(
            r#"{"id":"deepseek","name":"DeepSeek","builtin":true,
                "endpoints":[{"apiMode":"anthropic","baseUrl":"https://api.deepseek.com/anthropic"}],
                "defaultModel":"deepseek-flash","credentialRef":"provider/deepseek","proxy":null}"#,
        )
        .unwrap();
        assert!(provider.enabled, "旧数据供应商默认启用");
        assert!(provider.builtin_models.is_empty());
        assert!(provider.disabled_models.is_empty());

        let model: ModelConfig = serde_json::from_str(
            r#"{"id":"m1","name":"测试","baseUrl":"https://api.deepseek.com/anthropic",
                "model":"deepseek-flash","credentialRef":"provider/deepseek","proxy":null,
                "providerId":"deepseek","compatibility":"deepSeek","apiMode":"anthropic",
                "maxOutputTokens":null}"#,
        )
        .unwrap();
        assert!(model.enabled, "旧数据模型默认启用");
        assert!(!model.builtin);
        assert_eq!(model.effort, None);
        assert!(model.efforts.is_empty());
    }

    #[test]
    fn effort_lists_roundtrip() {
        let model = ModelConfig {
            efforts: vec!["low".into(), "high".into()],
            effort: Some("high".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        let back: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.efforts, model.efforts);
        assert_eq!(back.effort, model.effort);
    }

    #[test]
    fn known_efforts_are_ordered_low_to_high() {
        assert_eq!(
            KNOWN_EFFORTS,
            ["disabled", "minimal", "low", "medium", "high", "max"]
        );
    }

    /// 上下文窗口：旧 JSON 缺省为 None；新 JSON 以 camelCase `contextWindow` 序列化。
    #[test]
    fn context_window_defaults_and_roundtrip() {
        let legacy: ModelConfig = serde_json::from_str(
            r#"{"id":"m1","name":"测试","baseUrl":"https://api.deepseek.com",
                "model":"deepseek-flash","credentialRef":"provider/deepseek","proxy":null,
                "providerId":"deepseek","compatibility":"deepSeek","apiMode":"anthropic",
                "maxOutputTokens":null}"#,
        )
        .unwrap();
        assert_eq!(legacy.context_window, None, "旧数据模型窗口缺省为 None");

        let builtin: BuiltinModel =
            serde_json::from_str(r#"{"id":"glm-4.7","name":"GLM-4.7","efforts":["low","high"]}"#)
                .unwrap();
        assert_eq!(
            builtin.context_window, None,
            "旧内置模型条目窗口缺省为 None"
        );
        assert_eq!(
            builtin.max_output_tokens, None,
            "旧内置模型条目输出上限缺省为 None"
        );

        let model = ModelConfig {
            context_window: Some(200_000),
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("\"contextWindow\":200000"));
        let back: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.context_window, Some(200_000));
    }

    /// 新增 auth 字段对旧 JSON 缺省为 ApiKey；ChatGptOAuth 以 camelCase 序列化。
    #[test]
    fn provider_auth_defaults_and_roundtrip() {
        let provider: ProviderInfo = serde_json::from_str(
            r#"{"id":"chatgpt","name":"ChatGPT","builtin":true,
                "endpoints":[{"apiMode":"responses","baseUrl":"https://chatgpt.com/backend-api/codex"}],
                "defaultModel":"gpt-5.5","credentialRef":"provider/chatgpt","proxy":null,
                "auth":"chatGptOAuth"}"#,
        )
        .unwrap();
        assert_eq!(provider.auth, ProviderAuth::ChatGptOAuth);

        let legacy: ProviderInfo = serde_json::from_str(
            r#"{"id":"deepseek","name":"DeepSeek","builtin":true,
                "endpoints":[],"defaultModel":"deepseek-flash",
                "credentialRef":"provider/deepseek","proxy":null}"#,
        )
        .unwrap();
        assert_eq!(legacy.auth, ProviderAuth::ApiKey);

        let model = ModelConfig {
            auth: ProviderAuth::ChatGptOAuth,
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("\"auth\":\"chatGptOAuth\""));
    }
}
