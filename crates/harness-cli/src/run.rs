//! `harness run`：一次性执行 Agent 任务（docs/specs/cli-plan.md）。
use crate::{
    approval::{self, Approval, Decision},
    console::{Console, RunResult},
    diffview::ColorChoice,
    scope::{Phase, RunScope, Verdict},
};
use harness_contracts::{AppError, AppResult, ApprovalRequest, LiveEvent, SessionEvent};
use harness_runtime::Runtime;
use std::{
    collections::HashMap,
    io::{BufRead, IsTerminal, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    sync::broadcast::error::RecvError,
    time::{Duration, Instant, timeout},
};

fn io_err(error: std::io::Error) -> AppError {
    AppError::new("IO", error.to_string())
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 任务提示词；缺省时按 stdin 类型处理（管道读至 EOF，TTY 报错）
    pub prompt: Option<String>,
    /// 目标项目目录（默认当前目录）
    #[arg(short = 'C', long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    /// 模型 ID（默认 settings 的 cli.default_model，否则报错列出可用模型）
    #[arg(short = 'm', long)]
    pub model: Option<String>,
    /// 显式指定工作区（与 --session 互斥）
    #[arg(long, conflicts_with = "session")]
    pub workspace: Option<String>,
    /// 续已有会话
    #[arg(long)]
    pub session: Option<String>,
    /// 从 stdin 读取提示词至 EOF（--prompt -）
    #[arg(long = "prompt", value_name = "SOURCE")]
    pub prompt_stdin: Option<String>,
    /// JSONL 事件流输出到 stdout
    #[arg(long)]
    pub json: bool,
    /// 审批策略；缺省时 stdin 为 TTY 取 ask，否则报错
    #[arg(long)]
    pub approval: Option<Approval>,
    /// 颜色输出
    #[arg(long, default_value = "auto")]
    pub color: ColorChoice,
}

/// 事件泵信号。
enum Signal {
    /// 本消息回合已结算（turn.end），携带 status
    ParentClosed(String),
    /// 本消息在排队时被取消（inbox.cancelled）
    Cancelled,
}

pub async fn execute(runtime: Arc<Runtime>, args: Args) -> AppResult<i32> {
    let stdin_tty = std::io::stdin().is_terminal();
    let policy = approval::resolve_policy(args.approval, stdin_tty)?;
    if let Some(source) = &args.prompt_stdin {
        if source != "-" {
            return Err(AppError::new(
                "ARGS",
                "--prompt 仅支持 -（从 stdin 读取至 EOF）",
            ));
        }
        if policy == Approval::Ask {
            return Err(AppError::new(
                "APPROVAL_TTY",
                "--prompt - 会占用 stdin，无法用于 ask 审批；请改用 --approval auto|never",
            ));
        }
    }
    if args.prompt.is_some() && args.prompt_stdin.is_some() {
        return Err(AppError::new("ARGS", "位置提示词与 --prompt - 只能二选一"));
    }
    let prompt = if let Some(prompt) = &args.prompt {
        prompt.clone()
    } else if args.prompt_stdin.is_some() {
        read_stdin_to_string()?
    } else if stdin_tty {
        return Err(AppError::new(
            "ARGS",
            "缺少提示词：请作为参数传入，或经管道从 stdin 提供",
        ));
    } else {
        read_stdin_to_string()?
    };
    if prompt.trim().is_empty() {
        return Err(AppError::new("INVALID_INPUT", "提示词不能为空"));
    }

    let (workspace_id, session_id, reuse_warning) =
        resolve_target(&runtime, &args, &prompt).await?;
    let model_id = resolve_model(&runtime, args.model.clone()).await?;
    let provider = runtime.models.build_model(&model_id).await?;

    let mut console = Console::new(args.json, &session_id, args.color);
    if let Some(warning) = reuse_warning {
        console.note(&warning).map_err(io_err)?;
    }

    // 订阅必须先于入队，避免丢失本消息的 inbox.queued/turn.start 事件
    let mut events = runtime.subscribe();
    let message_id = runtime
        .enqueue_message(&session_id, prompt.clone(), vec![])
        .await?;
    let mut scope = RunScope::new(&session_id, &message_id);
    let mut task = tokio::spawn({
        let runtime = runtime.clone();
        let session = session_id.clone();
        let message = message_id.clone();
        let provider = provider.clone();
        async move { runtime.execute_message(session, message, provider).await }
    });

    let mut last_seq: HashMap<String, u64> = HashMap::new();
    let mut closed_status: Option<String> = None;
    let mut user_abort = false;
    let mut cancel_requested_at: Option<Instant> = None;
    loop {
        if let Some(at) = cancel_requested_at
            && at.elapsed() > Duration::from_secs(10)
        {
            user_abort = true;
            break;
        }
        let signal = tokio::select! {
            event = events.recv() => match event {
                Ok(LiveEvent::Durable { event }) => {
                    handle_durable(&runtime, &mut scope, &mut console, policy, &event, &mut last_seq).await?
                }
                Ok(LiveEvent::Delta { session_id, request_id, index, text, reasoning }) => {
                    if scope.in_scope(&session_id) {
                        console.delta(&session_id, &request_id, index, &text, reasoning).map_err(io_err)?;
                    }
                    None
                }
                Ok(LiveEvent::Status { .. }) => None,
                Err(RecvError::Lagged(skipped)) => {
                    resync(&runtime, &mut scope, &mut console, policy, &mut last_seq, skipped).await?;
                    None
                }
                Err(RecvError::Closed) => return Err(AppError::new("EVENTS", "事件通道已关闭")),
            },
            _ = tokio::signal::ctrl_c() => {
                if cancel_requested_at.is_some() {
                    user_abort = true;
                    break;
                }
                cancel_requested_at = Some(Instant::now());
                match scope.phase {
                    Phase::Pending => runtime.cancel_message(&session_id, &message_id),
                    _ => runtime.cancel(&session_id),
                }
                console.note("已请求取消，等待回合结算（≤10 秒；再次 Ctrl+C 立即退出）").map_err(io_err)?;
                None
            }
        };
        match signal {
            Some(Signal::ParentClosed(status)) => closed_status = Some(status),
            Some(Signal::Cancelled) => closed_status = Some("cancelled".into()),
            None => {}
        }
        if closed_status.is_some() || user_abort {
            break;
        }
    }

    let (status, exit_code) = if user_abort {
        ("cancelled".to_owned(), 130)
    } else {
        match timeout(Duration::from_secs(10), &mut task).await {
            Ok(Ok(Ok(()))) => ("completed".to_owned(), 0),
            Ok(Ok(Err(error))) if error.code == "CANCELLED" => ("cancelled".to_owned(), 130),
            Ok(Ok(Err(error))) => {
                console.note(&format!("{error}")).map_err(io_err)?;
                ("error".to_owned(), 1)
            }
            Ok(Err(_)) => ("error".to_owned(), 1),
            Err(_) => ("cancelled".to_owned(), 130),
        }
    };

    // 子代理收尾：父回合结束后等待作用域内子会话结束（≤600 秒）
    for child in runtime.children_of(&session_id) {
        scope.sessions.insert(child);
    }
    drain_children(
        &runtime,
        &mut events,
        &mut scope,
        &mut console,
        policy,
        &mut last_seq,
    )
    .await?;

    let changed_files = runtime
        .workspaces
        .diff(&workspace_id)
        .map(|changes| changes.len())
        .unwrap_or(0);
    console
        .result(&RunResult {
            message_id: message_id.clone(),
            run_id: scope.claimed_run_id().map(str::to_owned),
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            status: status.clone(),
            exit_code,
            final_message: console.last_assistant().to_owned(),
            changed_files,
        })
        .map_err(io_err)?;
    Ok(exit_code)
}

#[allow(clippy::too_many_arguments)]
async fn handle_durable(
    runtime: &Arc<Runtime>,
    scope: &mut RunScope,
    console: &mut Console,
    policy: Approval,
    event: &SessionEvent,
    last_seq: &mut HashMap<String, u64>,
) -> AppResult<Option<Signal>> {
    if let Ok(seq) = event.seq.parse::<u64>() {
        last_seq.insert(event.session_id.clone(), seq);
    }
    let verdict = scope.observe(event);
    if verdict == Verdict::Ignored {
        return Ok(None);
    }
    console.durable(event).map_err(io_err)?;
    if event.kind == "approval.request"
        && let Ok(request) = serde_json::from_value::<ApprovalRequest>(event.data.clone())
        && scope.in_scope(&request.session_id)
    {
        resolve_approval(runtime, console, policy, &request)?;
    }
    Ok(match verdict {
        Verdict::TurnClosed(status) => Some(Signal::ParentClosed(status)),
        Verdict::Cancelled => Some(Signal::Cancelled),
        _ => None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn resync(
    runtime: &Arc<Runtime>,
    scope: &mut RunScope,
    console: &mut Console,
    policy: Approval,
    last_seq: &mut HashMap<String, u64>,
    skipped: u64,
) -> AppResult<()> {
    scope.note_lagged();
    for session in scope.sessions.clone() {
        let from = last_seq.get(&session).copied().unwrap_or(0);
        let missed = runtime
            .store
            .events(&session, from)
            .await
            .unwrap_or_default();
        let mut to = from;
        for event in missed {
            if let Ok(seq) = event.seq.parse::<u64>() {
                to = to.max(seq);
            }
            handle_durable(runtime, scope, console, policy, &event, last_seq).await?;
        }
        console
            .gap(&session, from + 1, to.max(from + skipped))
            .map_err(io_err)?;
    }
    Ok(())
}

fn resolve_approval(
    runtime: &Runtime,
    console: &mut Console,
    policy: Approval,
    request: &ApprovalRequest,
) -> AppResult<()> {
    let allow = match Decision::from(policy) {
        Decision::Allow => true,
        Decision::Deny => false,
        Decision::Ask => {
            console.approval(request).map_err(io_err)?;
            read_confirmation()
        }
    };
    runtime.approvals.resolve(&request.id, allow)
}

fn read_confirmation() -> bool {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
    }
}

fn read_stdin_to_string() -> AppResult<String> {
    let mut text = String::new();
    std::io::stdin()
        .lock()
        .read_to_string(&mut text)
        .map_err(|e| AppError::new("STDIN", e.to_string()))?;
    Ok(text)
}

/// 工作区与会话解析：--session 绑定校验；按 source_path 复用最新工作区；脏副本拒绝。
async fn resolve_target(
    runtime: &Arc<Runtime>,
    args: &Args,
    prompt: &str,
) -> AppResult<(String, String, Option<String>)> {
    if let Some(session_id) = &args.session {
        let info = runtime.store.session(session_id).await?;
        let workspace = runtime.workspaces.get(&info.workspace_id)?;
        if let Some(cwd) = &args.cwd {
            let want = canonical(cwd)?;
            if !workspace.source_path.eq_ignore_ascii_case(&want) {
                return Err(AppError::new(
                    "SESSION_CWD_MISMATCH",
                    format!(
                        "--session 绑定的工作区目录为 {}，与 --cwd {} 不一致",
                        workspace.source_path, want
                    ),
                ));
            }
        }
        return Ok((workspace.id.clone(), session_id.clone(), None));
    }
    let cwd = canonical(args.cwd.as_deref().unwrap_or(Path::new(".")))?;
    let candidate = runtime
        .workspaces
        .list()?
        .into_iter()
        .rev()
        .find(|workspace| workspace.source_path.eq_ignore_ascii_case(&cwd));
    let (workspace, warning) = match (&args.workspace, candidate) {
        (Some(id), _) => (
            runtime.workspaces.get(id)?,
            Some(format!(
                "警告：显式复用工作区 {id}；若副本存在未应用改动，将混入本次任务上下文"
            )),
        ),
        (None, Some(workspace)) => {
            let changes = runtime.workspaces.diff(&workspace.id)?;
            if !changes.is_empty() {
                return Err(AppError::new(
                    "WORKSPACE_DIRTY",
                    format!(
                        "工作副本存在 {} 项未应用改动：请先 harness diff {}/apply 处理，或用 --workspace <id>/--session <id> 显式复用",
                        changes.len(),
                        workspace.id
                    ),
                ));
            }
            (workspace, None)
        }
        (None, None) => (runtime.workspaces.create(Path::new(&cwd))?, None),
    };
    let title: String = prompt.trim().chars().take(24).collect();
    let session = runtime
        .store
        .create_session(
            &workspace.id,
            if title.is_empty() {
                "CLI 会话"
            } else {
                &title
            },
        )
        .await?;
    Ok((workspace.id.clone(), session.id.clone(), warning))
}

async fn resolve_model(runtime: &Arc<Runtime>, explicit: Option<String>) -> AppResult<String> {
    if let Some(id) = explicit {
        return Ok(id);
    }
    if let Some(default) = runtime
        .store
        .get("cli.default_model")
        .await?
        .and_then(|value| value.as_str().map(str::to_owned))
    {
        return Ok(default);
    }
    let models = runtime.models.list_models().await?;
    let listing = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(AppError::new(
        "MODEL_NOT_SELECTED",
        format!("未选择模型：用 -m <id> 或 harness models default <id>；可用模型：{listing}"),
    ))
}

fn canonical(path: &Path) -> AppResult<String> {
    dunce::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| AppError::new("PATH", e.to_string()))
}

/// 父回合结束后的子代理收尾：继续渲染与审批，直到作用域内无活动子会话（≤600 秒）。
async fn drain_children(
    runtime: &Arc<Runtime>,
    events: &mut tokio::sync::broadcast::Receiver<LiveEvent>,
    scope: &mut RunScope,
    console: &mut Console,
    policy: Approval,
    last_seq: &mut HashMap<String, u64>,
) -> AppResult<()> {
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        let children_active = scope
            .sessions
            .iter()
            .any(|session| runtime.is_active(session) && session != scope.parent());
        if !children_active {
            return Ok(());
        }
        tokio::select! {
            event = events.recv() => match event {
                Ok(LiveEvent::Durable { event }) => {
                    handle_durable(runtime, scope, console, policy, &event, last_seq).await?;
                }
                Ok(LiveEvent::Delta { session_id, request_id, index, text, reasoning }) => {
                    if scope.in_scope(&session_id) {
                        console.delta(&session_id, &request_id, index, &text, reasoning).map_err(io_err)?;
                    }
                }
                Ok(LiveEvent::Status { .. }) => {}
                Err(RecvError::Lagged(skipped)) => {
                    resync(runtime, scope, console, policy, last_seq, skipped).await?;
                }
                Err(RecvError::Closed) => {
                    return Err(AppError::new("EVENTS", "事件通道已关闭"));
                }
            },
            _ = tokio::signal::ctrl_c() => {
                for session in &scope.sessions {
                    runtime.cancel(session);
                }
                console.note("已请求取消全部作用域内回合").map_err(io_err)?;
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            _ = tokio::time::sleep_until(deadline) => {
                console.note("子代理收尾达到 600 秒上限，继续退出").map_err(io_err)?;
                return Ok(());
            }
        }
    }
}
