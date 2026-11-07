//! Durable jobs, scoped agent messaging and application-lifetime scheduling.
mod terminals;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone, Utc};
use harness_contracts::*;
use harness_platform::ShellService;
use harness_runtime::{
    ExternalTool, Registration, Runtime, ToolContext,
    model::{ModelMessage, ModelProvider},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
pub use terminals::{PlatformWorld, PtyWorld, TerminalSession};
use terminals::{TERMINAL_LIMIT, TerminalEntry};
use tokio_util::sync::CancellationToken;
const SCHEDULE_KEY: &str = "orchestration.schedules.v1";
const AUTOMATION_KEY: &str = "orchestration.automations.v1";
const AUTOMATION_LIMIT: usize = 1000;
const AUTOMATION_RUNS_KEEP: usize = 20;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub session_id: String,
    pub workspace_id: String,
    pub at: String,
    pub task: String,
    pub model_id: String,
    pub status: String,
}
/// 自动化任务的周期类型；单次固定时刻，其余按本地日历循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutomationKind {
    Once,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}
/// 自动化任务的一次触发记录；内嵌于 [`Automation`]，仅保留最近若干条。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRun {
    pub id: String,
    pub scheduled_at: Option<String>,
    pub triggered_at: String,
    pub trigger: String,
    pub status: String,
    pub session_id: String,
    pub error: Option<String>,
}
/// 用户通过桌面面板创建与管理的周期自动化任务；执行复用 [`run_owned`] 回合通道。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub session_id: Option<String>,
    pub workspace_id: String,
    pub kind: AutomationKind,
    pub at: Option<String>,
    pub time: Option<String>,
    pub weekdays: Option<Vec<u8>>,
    pub month_day: Option<u8>,
    pub month: Option<u8>,
    pub enabled: bool,
    pub status: String,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub run_count: u64,
    pub runs: Vec<AutomationRun>,
    pub created_at: String,
    pub updated_at: String,
}
/// 创建/编辑自动化任务的参数；`at` 为 RFC3339（once），`time` 为 "HH:MM"（其余周期）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationParams {
    pub workspace_id: String,
    pub title: Option<String>,
    pub prompt: String,
    pub session_id: Option<String>,
    pub kind: AutomationKind,
    pub at: Option<String>,
    pub time: Option<String>,
    pub weekdays: Option<Vec<u8>>,
    pub month_day: Option<u8>,
    pub month: Option<u8>,
}
/// Merged human view of one background shell job, derived from durable session events.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: String,
    pub command: String,
    pub status: String,
    pub cancel_requested: bool,
    pub output: Option<Value>,
    pub error: Option<Value>,
    pub created_at: String,
}
struct State {
    runtime: Weak<Runtime>,
    stop: CancellationToken,
    jobs: Mutex<HashMap<String, (String, CancellationToken)>>,
    agents: Mutex<HashMap<String, usize>>,
    terminals: Mutex<HashMap<String, Arc<TerminalEntry>>>,
    world: Arc<dyn PtyWorld>,
    schedules_lock: tokio::sync::Mutex<()>,
    ready: tokio::sync::OnceCell<()>,
}
pub struct Installed {
    _registrations: Vec<Registration>,
    state: Arc<State>,
}
impl Installed {
    pub fn has_activity(&self) -> bool {
        !self.state.jobs.lock().unwrap().is_empty() || !self.state.agents.lock().unwrap().is_empty()
    }
    fn runtime(&self) -> AppResult<Arc<Runtime>> {
        self.state
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))
    }
    /// Human panel: merged background shell job view for one session.
    pub async fn session_jobs(&self, session: &str) -> AppResult<Vec<JobInfo>> {
        let r = self.runtime()?;
        self.state.session_jobs(&r, session).await
    }
    /// Human panel: request cancellation of a session-owned background job.
    pub async fn cancel_job(&self, session: &str, id: &str) -> AppResult<()> {
        let r = self.runtime()?;
        self.state.cancel_job(&r, session, id).await.map(|_| ())
    }
    /// Human panel: schedules owned by one session.
    pub async fn session_schedules(&self, session: &str) -> AppResult<Vec<Schedule>> {
        // Still guard against a shut-down host; the schedule list itself is session-scoped state.
        self.runtime()?;
        Ok(self
            .state
            .schedules()
            .await?
            .into_iter()
            .filter(|s| s.session_id == session)
            .collect())
    }
    /// Human panel: delete a session-owned schedule and record the event.
    pub async fn delete_schedule(&self, session: &str, id: &str) -> AppResult<()> {
        let r = self.runtime()?;
        self.state.delete_schedule(&r, session, id).await
    }
    /// Human panel: automations of one workspace, newest first.
    pub async fn automation_list(&self, workspace: &str) -> AppResult<Vec<Automation>> {
        self.runtime()?;
        Ok(self
            .state
            .automations()
            .await?
            .into_iter()
            .filter(|a| a.workspace_id == workspace)
            .collect())
    }
    /// Human panel: create an automation from validated panel input.
    pub async fn automation_create(&self, p: AutomationParams) -> AppResult<Automation> {
        let r = self.runtime()?;
        validate_automation(&p)?;
        let _lock = self.state.schedules_lock.lock().await;
        let mut all = self.state.automations().await?;
        if all.len() >= AUTOMATION_LIMIT {
            return Err(AppError::new("AUTOMATION_LIMIT", "自动化任务最多1000项"));
        }
        self.state
            .check_automation_session(&r, p.session_id.as_deref(), &p.workspace_id)
            .await?;
        let at_utc = p
            .kind
            .eq(&AutomationKind::Once)
            .then(|| automation_at_utc(&p));
        let mut a = Automation {
            id: new_id(),
            title: p.title.unwrap_or_default(),
            prompt: p.prompt,
            session_id: p.session_id,
            workspace_id: p.workspace_id,
            kind: p.kind,
            at: at_utc,
            time: p.time,
            weekdays: p.weekdays,
            month_day: p.month_day,
            month: p.month,
            enabled: true,
            status: "active".into(),
            next_run_at: None,
            last_run_at: None,
            run_count: 0,
            runs: Vec::new(),
            created_at: now(),
            updated_at: now(),
        };
        a.next_run_at = automation_next_run_at(&Local, &a, Local::now()).map(|t| t.to_rfc3339());
        all.insert(0, a.clone());
        self.state.save_automations(&all).await?;
        Ok(a)
    }
    /// Human panel: replace fields of one automation and recompute the next run.
    pub async fn automation_update(&self, id: &str, p: AutomationParams) -> AppResult<Automation> {
        let r = self.runtime()?;
        validate_automation(&p)?;
        let _lock = self.state.schedules_lock.lock().await;
        let mut all = self.state.automations().await?;
        let Some(a) = all.iter_mut().find(|x| x.id == id) else {
            return Err(AppError::new("NOT_FOUND", "自动化任务不存在"));
        };
        self.state
            .check_automation_session(&r, p.session_id.as_deref(), &p.workspace_id)
            .await?;
        let at_utc = p
            .kind
            .eq(&AutomationKind::Once)
            .then(|| automation_at_utc(&p));
        a.title = p.title.unwrap_or_default();
        a.prompt = p.prompt;
        a.session_id = p.session_id;
        a.kind = p.kind;
        a.at = at_utc;
        a.time = p.time;
        a.weekdays = p.weekdays;
        a.month_day = p.month_day;
        a.month = p.month;
        a.updated_at = now();
        a.next_run_at = automation_next_run_at(&Local, a, Local::now()).map(|t| t.to_rfc3339());
        // 编辑把未来触发点带回来的任务复活；paused 保持暂停，等用户手动启用。
        if a.status != "paused" {
            a.status = "active".into();
        }
        let updated = a.clone();
        self.state.save_automations(&all).await?;
        Ok(updated)
    }
    /// Human panel: remove one automation.
    pub async fn automation_delete(&self, id: &str) -> AppResult<()> {
        let _lock = self.state.schedules_lock.lock().await;
        let mut all = self.state.automations().await?;
        let before = all.len();
        all.retain(|x| x.id != id);
        if all.len() == before {
            return Err(AppError::new("NOT_FOUND", "自动化任务不存在"));
        }
        self.state.save_automations(&all).await
    }
    /// Human panel: pause or resume; resuming recomputes an overdue next run instead of catching up.
    pub async fn automation_set_enabled(&self, id: &str, enabled: bool) -> AppResult<Automation> {
        let _lock = self.state.schedules_lock.lock().await;
        let mut all = self.state.automations().await?;
        let Some(a) = all.iter_mut().find(|x| x.id == id) else {
            return Err(AppError::new("NOT_FOUND", "自动化任务不存在"));
        };
        a.enabled = enabled;
        a.updated_at = now();
        if !enabled {
            a.status = "paused".into();
        } else if a.status == "paused" {
            a.status = "active".into();
            a.next_run_at = automation_next_run_at(&Local, a, Local::now()).map(|t| t.to_rfc3339());
        }
        let updated = a.clone();
        self.state.save_automations(&all).await?;
        Ok(updated)
    }
    /// Human panel: dispatch one manual run immediately; never moves the planned schedule.
    pub async fn automation_run_now(&self, id: &str) -> AppResult<Automation> {
        let r = self.runtime()?;
        let _lock = self.state.schedules_lock.lock().await;
        let mut all = self.state.automations().await?;
        let Some(a) = all.iter_mut().find(|x| x.id == id) else {
            return Err(AppError::new("NOT_FOUND", "自动化任务不存在"));
        };
        let provider = automation_provider_of(&r).await?;
        let session_id = self.state.automation_session(&r, a).await?;
        let run = AutomationRun {
            id: new_id(),
            scheduled_at: None,
            triggered_at: now(),
            trigger: "manual".into(),
            status: "running".into(),
            session_id: session_id.clone(),
            error: None,
        };
        a.runs.insert(0, run.clone());
        a.runs.truncate(AUTOMATION_RUNS_KEEP);
        a.run_count += 1;
        a.last_run_at = Some(now());
        let updated = a.clone();
        self.state.save_automations(&all).await?;
        drop(_lock);
        let automation_id = updated.id.clone();
        let run_id = run.id.clone();
        let state = self.state.clone();
        let rt = r.clone();
        let stop = self.state.stop.child_token();
        tokio::spawn(async move {
            state
                .dispatch_automation(rt, automation_id, run_id, session_id, provider, stop)
                .await;
        });
        Ok(updated)
    }
}
impl Drop for Installed {
    fn drop(&mut self) {
        self.state.stop.cancel();
        for (_, token) in self.state.jobs.lock().unwrap().values() {
            token.cancel();
        }
        for entry in self.state.terminals.lock().unwrap().values() {
            entry.token.cancel();
        }
        if let Some(runtime) = self.state.runtime.upgrade() {
            for id in self.state.agents.lock().unwrap().keys() {
                runtime.cancel(id);
            }
        }
    }
}
/// Keep the returned owner alive for the entire application lifetime.
pub fn install(runtime: Arc<Runtime>) -> AppResult<Installed> {
    install_with(runtime, Arc::new(terminals::PlatformWorld))
}
/// Same as [`install`] with an injected execution world (tests, future remote worlds).
pub fn install_with(runtime: Arc<Runtime>, world: Arc<dyn PtyWorld>) -> AppResult<Installed> {
    let state = Arc::new(State {
        runtime: Arc::downgrade(&runtime),
        stop: CancellationToken::new(),
        jobs: Mutex::new(HashMap::new()),
        agents: Mutex::new(HashMap::new()),
        terminals: Mutex::new(HashMap::new()),
        world,
        schedules_lock: tokio::sync::Mutex::new(()),
        ready: tokio::sync::OnceCell::new(),
    });
    let mut registrations = Vec::new();
    for name in [
        "background_run",
        "background_status",
        "background_cancel",
        "agent_wait",
        "agent_send",
        "agent_fork",
        "schedule_write",
        "schedule_list",
        "schedule_delete",
        "terminal_open",
        "terminal_write",
        "terminal_read",
        "terminal_close",
    ] {
        registrations.push(runtime.register_tool(Arc::new(Tool {
            state: state.clone(),
            name: name.into(),
        }))?);
    }
    let ticker = state.clone();
    tokio::spawn(async move {
        if ticker
            .ready
            .get_or_try_init(|| ticker.recover_all())
            .await
            .is_err()
        {
            return;
        }
        let mut first = true;
        loop {
            if ticker.stop.is_cancelled() {
                break;
            }
            if let Err(e) = ticker.tick(first).await
                && let Some(r) = ticker.runtime.upgrade()
                && let Ok(sessions) = r.store.sessions(None).await
                && let Some(s) = sessions.first()
            {
                let _ = r.append(&s.id, "schedule.error", json!({"error":e})).await;
            }
            if let Err(e) = ticker.tick_automations(first).await
                && let Some(r) = ticker.runtime.upgrade()
                && let Ok(sessions) = r.store.sessions(None).await
                && let Some(s) = sessions.first()
            {
                let _ = r
                    .append(&s.id, "automation.error", json!({"error":e}))
                    .await;
            }
            first = false;
            tokio::select! {_=ticker.stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(250))=>{}}
        }
    });
    Ok(Installed {
        _registrations: registrations,
        state,
    })
}
struct Tool {
    state: Arc<State>,
    name: String,
}
fn string<'a>(v: &'a Value, k: &str) -> AppResult<&'a str> {
    v[k].as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 1024 * 1024)
        .ok_or_else(|| AppError::new("TOOL_ARGUMENTS", format!("缺少有效字段 {k}")))
}
fn error(e: impl std::fmt::Display) -> AppError {
    AppError::new("ORCHESTRATION", e.to_string())
}
fn scope(runtime: &Runtime, parent: &str, target: &str) -> AppResult<()> {
    if runtime.children_of(parent).iter().any(|s| s == target) {
        Ok(())
    } else {
        Err(AppError::new("AGENT_SCOPE", "只能访问本会话创建的子代理"))
    }
}
fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let (h, m) = (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}
/// 在指定时区构造 `date` 当天 `HH:MM` 的时刻；DST 缺失该时刻时返回 None（视为该日无此时刻）。
fn tz_at<Tz: TimeZone>(tz: &Tz, date: NaiveDate, h: u32, m: u32) -> Option<DateTime<Tz>> {
    tz.from_local_datetime(&date.and_hms_opt(h, m, 0)?).single()
}
/// 计算自动化任务自 `from`（按 `tz` 本地日历解释）起的下一次触发时刻（UTC）。
///
/// 缺日跳过：短月无 `month_day`、平年无 2/29 时继续搜索后续月份；单次任务直接返回其固定时刻。
pub fn automation_next_run_at<Tz: TimeZone>(
    tz: &Tz,
    a: &Automation,
    from: DateTime<Tz>,
) -> Option<DateTime<Utc>> {
    match a.kind {
        AutomationKind::Once => {
            a.at.as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
        }
        AutomationKind::Daily => {
            let (h, m) = parse_hhmm(a.time.as_deref()?)?;
            for days in 0..=1u64 {
                let date = from.date_naive().checked_add_days(Days::new(days))?;
                if let Some(t) = tz_at(tz, date, h, m)
                    && t > from
                {
                    return Some(t.with_timezone(&Utc));
                }
            }
            None
        }
        AutomationKind::Weekly => {
            let (h, m) = parse_hhmm(a.time.as_deref()?)?;
            let weekdays = a.weekdays.as_ref()?;
            for days in 0..8u64 {
                let date = from.date_naive().checked_add_days(Days::new(days))?;
                // number_from_monday: 1=周一..7=周日，与 weekdays 存储一致。
                if !weekdays.contains(&(date.weekday().number_from_monday() as u8)) {
                    continue;
                }
                if let Some(t) = tz_at(tz, date, h, m)
                    && t > from
                {
                    return Some(t.with_timezone(&Utc));
                }
            }
            None
        }
        AutomationKind::Monthly => {
            let (h, m) = parse_hhmm(a.time.as_deref()?)?;
            let day = a.month_day?;
            let (mut year, mut month) = (from.year(), from.month());
            for _ in 0..48 {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day as u32)
                    && let Some(t) = tz_at(tz, date, h, m)
                    && t > from
                {
                    return Some(t.with_timezone(&Utc));
                }
                if month == 12 {
                    year += 1;
                    month = 1;
                } else {
                    month += 1;
                }
            }
            None
        }
        AutomationKind::Yearly => {
            let (h, m) = parse_hhmm(a.time.as_deref()?)?;
            let (month, day) = (a.month?, a.month_day?);
            let (mut year, mut mon) = (from.year(), from.month());
            for _ in 0..120 {
                if mon == month as u32
                    && let Some(date) = NaiveDate::from_ymd_opt(year, mon, day as u32)
                    && let Some(t) = tz_at(tz, date, h, m)
                    && t > from
                {
                    return Some(t.with_timezone(&Utc));
                }
                if mon == 12 {
                    year += 1;
                    mon = 1;
                } else {
                    mon += 1;
                }
            }
            None
        }
    }
}
/// 校验创建/编辑参数；once 的时刻必须在未来，周期字段按类型域检查。
fn validate_automation(p: &AutomationParams) -> AppResult<()> {
    if p.prompt.trim().is_empty() || p.prompt.len() > 1024 * 1024 {
        return Err(AppError::new(
            "AUTOMATION_ARGUMENTS",
            "任务内容不能为空且不超过1MiB",
        ));
    }
    if p.title.as_deref().is_some_and(|t| t.len() > 512) {
        return Err(AppError::new("AUTOMATION_ARGUMENTS", "标题不得超过512字节"));
    }
    match p.kind {
        AutomationKind::Once => {
            let at =
                p.at.as_deref()
                    .ok_or_else(|| AppError::new("AUTOMATION_ARGUMENTS", "单次任务缺少执行时间"))?;
            let parsed = DateTime::parse_from_rfc3339(at).map_err(|e| {
                AppError::new("AUTOMATION_ARGUMENTS", format!("执行时间格式无效: {e}"))
            })?;
            if parsed <= Utc::now() {
                return Err(AppError::new("AUTOMATION_TIME", "执行时间必须在未来"));
            }
        }
        AutomationKind::Daily => {
            parse_hhmm(p.time.as_deref().unwrap_or_default()).ok_or_else(|| {
                AppError::new("AUTOMATION_ARGUMENTS", "时间必须为合法的 24 小时制 HH:MM")
            })?;
        }
        AutomationKind::Weekly => {
            let days = p.weekdays.as_deref().unwrap_or_default();
            if days.is_empty() || days.iter().any(|d| !(1..=7).contains(d)) {
                return Err(AppError::new(
                    "AUTOMATION_ARGUMENTS",
                    "周几必须为 1（周一）到 7（周日）的非空列表",
                ));
            }
        }
        AutomationKind::Monthly => {
            if !p.month_day.is_some_and(|d| (1..=31).contains(&d)) {
                return Err(AppError::new("AUTOMATION_ARGUMENTS", "几号必须为 1-31"));
            }
        }
        AutomationKind::Yearly => {
            if !p.month.is_some_and(|m| (1..=12).contains(&m))
                || !p.month_day.is_some_and(|d| (1..=31).contains(&d))
            {
                return Err(AppError::new(
                    "AUTOMATION_ARGUMENTS",
                    "年份日期必须为合法的月份（1-12）与几号（1-31）",
                ));
            }
        }
    }
    Ok(())
}
/// Resolve the execution provider: host default selection first, then the first enabled model.
async fn automation_provider_of(r: &Runtime) -> AppResult<Arc<dyn ModelProvider>> {
    if let Some(v) = r.store.get("app.default_selection").await?
        && let Ok(sel) = serde_json::from_value::<DefaultSelection>(v)
    {
        return r.models.build_model_with(&sel.model_id, sel.effort).await;
    }
    let models = r.models.list_models().await?;
    let first = models
        .iter()
        .find(|m| m.enabled)
        .ok_or_else(|| AppError::new("AUTOMATION_MODEL", "没有已启用的模型"))?;
    r.models.build_model(&first.id).await
}
/// 归一化 `at` 为 UTC RFC3339 存储。
fn automation_at_utc(p: &AutomationParams) -> String {
    DateTime::parse_from_rfc3339(p.at.as_deref().unwrap_or_default())
        .map(|t| t.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_default()
}
#[async_trait]
impl ExternalTool for Tool {
    fn definition(&self) -> ToolDefinition {
        let s = json!({"type":"string","minLength":1});
        let (properties, required, description) = match self.name.as_str() {
            "background_run" => (
                json!({"command":s}),
                vec!["command"],
                "在托管副本启动真实后台Shell任务，需审批",
            ),
            "background_status" => (
                json!({"id":s}),
                vec!["id"],
                "查询本会话持久后台任务状态和输出",
            ),
            "background_cancel" => (
                json!({"id":s}),
                vec!["id"],
                "取消本会话后台任务；待进程真正结束后释放工作区锁",
            ),
            "agent_wait" => (
                json!({"sessionId":s,"timeoutMs":{"type":"integer","minimum":0,"maximum":120000}}),
                vec!["sessionId"],
                "有界等待本会话子代理的完成事件",
            ),
            "agent_send" => (
                json!({"sessionId":s,"task":s}),
                vec!["sessionId", "task"],
                "向本会话子代理的串行收件队列投递任务",
            ),
            "agent_fork" => (
                json!({"sessionId":s}),
                vec!["sessionId"],
                "在回合边界复制本会话或直属子代理的历史，归属当前会话",
            ),
            "schedule_write" => (
                json!({"at":s,"task":s,"modelId":s}),
                vec!["at", "task", "modelId"],
                "创建持久单次定时任务；过期不补跑，执行仍遵守工具审批",
            ),
            "schedule_delete" => (json!({"id":s}), vec!["id"], "删除本会话的定时任务"),
            "terminal_open" => (
                json!({}),
                vec![],
                "在托管副本打开持久交互式PowerShell终端（ConPTY），返回终端id；交互式程序用terminal_*，一次性命令用shell；打开需审批且覆盖该终端后续输入，每会话最多4个",
            ),
            "terminal_write" => (
                json!({"id":s,"input":s,"yieldMs":{"type":"integer","minimum":250,"maximum":30000}}),
                vec!["id", "input"],
                "向本会话已打开终端写入命令并等待增量输出（ANSI已过滤）；终端存续期间写入不再逐次审批",
            ),
            "terminal_read" => (
                json!({"id":s,"yieldMs":{"type":"integer","minimum":250,"maximum":30000}}),
                vec!["id"],
                "读取本会话终端自上次读取以来的新输出，不写入",
            ),
            "terminal_close" => (
                json!({"id":s}),
                vec!["id"],
                "关闭本会话终端并终止其进程树，返回尾部输出blob",
            ),
            _ => (json!({}), vec![], "列出本会话的定时任务"),
        };
        ToolDefinition {
            name: self.name.clone(),
            description: description.into(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        }
    }
    fn requires_approval(&self) -> bool {
        !matches!(
            self.name.as_str(),
            "background_status"
                | "agent_wait"
                | "schedule_list"
                | "terminal_write"
                | "terminal_read"
                | "terminal_close"
        )
    }
    async fn execute(
        &self,
        context: ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        if cancel.is_cancelled() || self.state.stop.is_cancelled() {
            return Err(harness_runtime::model::cancelled());
        }
        let r = self
            .state
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))?;
        let session = r.store.session(&context.session_id).await?;
        if session.workspace_id != context.workspace_id {
            return Err(AppError::new("AGENT_SCOPE", "会话与工作区不匹配"));
        }
        let value = match self.name.as_str() {
            "background_run" => {
                let command = string(&args, "command")?.to_owned();
                let lease = r.workspaces.acquire(&context.workspace_id)?;
                let id = new_id();
                let token = self.state.stop.child_token();
                r.append(
                    &context.session_id,
                    "job.started",
                    json!({"id":id,"command":command,"status":"running"}),
                )
                .await?;
                self.state
                    .jobs
                    .lock()
                    .unwrap()
                    .insert(id.clone(), (context.session_id.clone(), token.clone()));
                let state = self.state.clone();
                let sid = context.session_id.clone();
                let job = id.clone();
                tokio::spawn(async move {
                    let result = ShellService::run(
                        std::path::Path::new(&lease.info.replica_path),
                        &command,
                        token.clone(),
                    )
                    .await;
                    // ShellService joins process-tree teardown before returning. Never drop the lease on cancellation early.
                    let status = if token.is_cancelled() {
                        "cancelled"
                    } else {
                        match &result {
                            Ok(o) if o.exit_code == 0 => "completed",
                            _ => "failed",
                        }
                    };
                    let payload = match result {
                        Ok(output) => json!({"id":job,"status":status,"output":output}),
                        Err(error) => json!({"id":job,"status":status,"error":error}),
                    };
                    let _ = r.append(&sid, "job.finished", payload).await;
                    drop(lease);
                    state.jobs.lock().unwrap().remove(&job);
                });
                json!({"id":id,"status":"running"})
            }
            "background_status" => {
                self.state
                    .job_status(&r, &context.session_id, string(&args, "id")?)
                    .await?
            }
            "background_cancel" => {
                self.state
                    .cancel_job(&r, &context.session_id, string(&args, "id")?)
                    .await?
            }
            "agent_fork" => {
                let source = string(&args, "sessionId")?;
                if source != context.session_id {
                    scope(&r, &context.session_id, source)?;
                }
                // Forking the calling session while its own turn is active is unsafe and is rejected by Runtime.
                if r.is_active(source) {
                    return Err(AppError::new("BUSY", "请在源会话回合结束后派生"));
                }
                let source_info = r.store.session(source).await?;
                if source_info.workspace_id != context.workspace_id {
                    return Err(AppError::new("AGENT_SCOPE", "子代理工作区不匹配"));
                }
                let fork = r.fork(source).await?;
                r.adopt_child(&context.session_id, &fork.id)?;
                r.append(
                    &context.session_id,
                    "subagent.created",
                    json!({"sessionId":fork.id,"source":source}),
                )
                .await?;
                json!({"sessionId":fork.id})
            }
            "agent_send" => {
                let id = string(&args, "sessionId")?.to_owned();
                scope(&r, &context.session_id, &id)?;
                let task = string(&args, "task")?.to_owned();
                r.append(
                    &context.session_id,
                    "subagent.message_sent",
                    json!({"sessionId":id,"content":task}),
                )
                .await?;
                let token = self.state.stop.child_token();
                *self
                    .state
                    .agents
                    .lock()
                    .unwrap()
                    .entry(id.clone())
                    .or_insert(0) += 1;
                let state = self.state.clone();
                let provider = context.provider.clone();
                let child = id.clone();
                tokio::spawn(async move {
                    let _ = run_owned(r, child.clone(), task, provider, token).await;
                    let mut agents = state.agents.lock().unwrap();
                    if let Some(n) = agents.get_mut(&child) {
                        *n -= 1;
                        if *n == 0 {
                            agents.remove(&child);
                        }
                    }
                });
                json!({"sessionId":id,"queued":true})
            }
            "agent_wait" => {
                let id = string(&args, "sessionId")?;
                scope(&r, &context.session_id, id)?;
                let timeout = args["timeoutMs"].as_u64().unwrap_or(30000).min(120000);
                let until = tokio::time::Instant::now() + Duration::from_millis(timeout);
                loop {
                    let events = r.store.events(id, 0).await?;
                    let start = events.iter().rposition(|e| e.kind == "turn.start");
                    let end = events.iter().rposition(|e| e.kind == "turn.end");
                    if !self.state.agents.lock().unwrap().contains_key(id)
                        && end.is_some_and(|e| start.is_none_or(|s| e > s))
                    {
                        let e = &events[end.unwrap()];
                        break json!({"sessionId":id,"status":e.data["status"],"events":events.iter().filter(|e|matches!(e.kind.as_str(),"assistant.message"|"turn.end")).collect::<Vec<_>>()});
                    }
                    if tokio::time::Instant::now() >= until {
                        break json!({"sessionId":id,"status":"waiting"});
                    }
                    tokio::select! {_=cancel.cancelled()=>return Err(harness_runtime::model::cancelled()),_=self.state.stop.cancelled()=>return Err(harness_runtime::model::cancelled()),_=tokio::time::sleep(Duration::from_millis(25))=>{}}
                }
            }
            "schedule_write" => {
                let at = string(&args, "at")?;
                let parsed = DateTime::parse_from_rfc3339(at).map_err(error)?;
                if parsed <= Utc::now() {
                    return Err(AppError::new("SCHEDULE_TIME", "执行时间必须在未来"));
                }
                let _lock = self.state.schedules_lock.lock().await;
                let mut schedules = self.state.schedules().await?;
                if schedules.len() >= 1000 {
                    return Err(AppError::new("SCHEDULE_LIMIT", "定时任务最多1000项"));
                }
                let schedule = Schedule {
                    id: new_id(),
                    session_id: context.session_id.clone(),
                    workspace_id: context.workspace_id.clone(),
                    at: parsed.to_rfc3339(),
                    task: string(&args, "task")?.into(),
                    model_id: string(&args, "modelId")?.into(),
                    status: "pending".into(),
                };
                schedules.push(schedule.clone());
                self.state.save(&schedules).await?;
                let v = serde_json::to_value(schedule).map_err(error)?;
                r.append(&context.session_id, "schedule.created", v.clone())
                    .await?;
                v
            }
            "schedule_delete" => {
                let id = string(&args, "id")?;
                self.state
                    .delete_schedule(&r, &context.session_id, id)
                    .await?;
                json!({"id":id,"deleted":true})
            }
            "schedule_list" => json!(
                self.state
                    .schedules()
                    .await?
                    .into_iter()
                    .filter(|s| s.session_id == context.session_id)
                    .collect::<Vec<_>>()
            ),
            "terminal_open" => {
                let count = self
                    .state
                    .terminals
                    .lock()
                    .unwrap()
                    .values()
                    .filter(|e| e.session_id == context.session_id)
                    .count();
                if count >= TERMINAL_LIMIT {
                    return Err(AppError::new("TERMINAL_LIMIT", "每个会话最多4个终端"));
                }
                let lease = r.workspaces.acquire(&context.workspace_id)?;
                let root = lease.info.replica_path.clone();
                let token = self.state.stop.child_token();
                let session = match self
                    .state
                    .world
                    .spawn(Path::new(&root), token.clone())
                    .await
                {
                    Ok(session) => session,
                    Err(e) => {
                        drop(lease);
                        return Err(e);
                    }
                };
                let id = new_id();
                r.append(
                    &context.session_id,
                    "terminal.opened",
                    json!({"id":id,"shell":"pwsh","status":"running"}),
                )
                .await?;
                self.state.terminals.lock().unwrap().insert(
                    id.clone(),
                    Arc::new(TerminalEntry {
                        session_id: context.session_id.clone(),
                        session: tokio::sync::Mutex::new(session),
                        _lease: lease,
                        token,
                    }),
                );
                json!({"id":id,"status":"running"})
            }
            "terminal_write" => {
                let id = string(&args, "id")?.to_owned();
                let input = string(&args, "input")?.to_owned();
                let yield_ms = args["yieldMs"].as_u64().unwrap_or(1000).clamp(250, 30000);
                let entry = self.state.terminal_entry(&id, &context.session_id)?;
                let mut session = entry.session.lock().await;
                session.write_input(&input).await?;
                let output = session.read_output(yield_ms).await?;
                let exit = session.exit_code().await;
                drop(session);
                if let Some(code) = exit {
                    self.state
                        .settle_terminal(&r, &context.session_id, &id, Some(code), "exited")
                        .await?;
                    json!({"id":id,"status":"exited","exitCode":code,"output":output})
                } else {
                    json!({"id":id,"status":"running","output":output})
                }
            }
            "terminal_read" => {
                let id = string(&args, "id")?.to_owned();
                let yield_ms = args["yieldMs"].as_u64().unwrap_or(1000).clamp(250, 30000);
                let entry = self.state.terminal_entry(&id, &context.session_id)?;
                let mut session = entry.session.lock().await;
                let output = session.read_output(yield_ms).await?;
                let exit = session.exit_code().await;
                drop(session);
                if let Some(code) = exit {
                    self.state
                        .settle_terminal(&r, &context.session_id, &id, Some(code), "exited")
                        .await?;
                    json!({"id":id,"status":"exited","exitCode":code,"output":output})
                } else {
                    json!({"id":id,"status":"running","output":output})
                }
            }
            "terminal_close" => {
                let id = string(&args, "id")?.to_owned();
                let entry = self.state.terminal_entry(&id, &context.session_id)?;
                entry.session.lock().await.terminate();
                let exit = tokio::time::timeout(Duration::from_secs(10), async {
                    let mut session = entry.session.lock().await;
                    session.exit_code().await
                })
                .await
                .ok()
                .flatten();
                let status = if exit.is_some() {
                    "exited"
                } else {
                    "terminated"
                };
                self.state
                    .settle_terminal(&r, &context.session_id, &id, exit, status)
                    .await?;
                json!({"id":id,"closed":true,"status":status})
            }
            _ => return Err(AppError::new("TOOL_UNAVAILABLE", &self.name)),
        };
        Ok(ToolResult {
            content: value.to_string(),
            is_error: false,
        })
    }
}
impl State {
    /// Resolve a terminal owned by the calling session.
    fn terminal_entry(&self, id: &str, session: &str) -> AppResult<Arc<TerminalEntry>> {
        self.terminals
            .lock()
            .unwrap()
            .get(id)
            .filter(|e| e.session_id == session)
            .cloned()
            .ok_or_else(|| AppError::new("TERMINAL_SCOPE", "终端不存在或不属于本会话"))
    }
    /// Settle a finished terminal: spill tail into a blob, append `terminal.closed`,
    /// drop the entry so the workspace lease is released.
    async fn settle_terminal(
        &self,
        r: &Runtime,
        session: &str,
        id: &str,
        exit: Option<i32>,
        status: &str,
    ) -> AppResult<()> {
        let entry = self.terminals.lock().unwrap().remove(id);
        let Some(entry) = entry else {
            return Ok(());
        };
        let (tail, truncated) = entry.session.lock().await.spill();
        let mut payload = json!({"id":id,"status":status});
        if let Some(code) = exit {
            payload["exitCode"] = code.into();
        }
        if !tail.trim().is_empty() {
            let hash = r.store.put_blob(tail.as_bytes()).await?;
            payload["outputBlobHash"] = hash.into();
            payload["outputTruncated"] = truncated.into();
        }
        r.append(session, "terminal.closed", payload).await?;
        Ok(())
    }
    async fn job_status(&self, r: &Runtime, session: &str, id: &str) -> AppResult<Value> {
        self.ready.get_or_try_init(|| self.recover_all()).await?;
        r.store
            .events(session, 0)
            .await?
            .into_iter()
            .rev()
            .find(|e| {
                matches!(e.kind.as_str(), "job.started" | "job.finished") && e.data["id"] == id
            })
            .map(|e| e.data)
            .ok_or_else(|| AppError::new("JOB_SCOPE", "任务不存在或不属于本会话"))
    }
    async fn session_jobs(&self, r: &Runtime, session: &str) -> AppResult<Vec<JobInfo>> {
        self.ready.get_or_try_init(|| self.recover_all()).await?;
        let mut jobs: Vec<JobInfo> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for e in r.store.events(session, 0).await? {
            let Some(id) = e.data["id"].as_str().map(str::to_owned) else {
                continue;
            };
            match e.kind.as_str() {
                "job.started" => {
                    index.insert(id.clone(), jobs.len());
                    jobs.push(JobInfo {
                        id,
                        command: e.data["command"].as_str().unwrap_or_default().into(),
                        status: "running".into(),
                        cancel_requested: false,
                        output: None,
                        error: None,
                        created_at: e.created_at.clone(),
                    });
                }
                "job.cancel_requested" => {
                    if let Some(pos) = index.get(&id) {
                        jobs[*pos].cancel_requested = true;
                    }
                }
                "job.finished" => {
                    if let Some(pos) = index.get(&id) {
                        let job = &mut jobs[*pos];
                        job.status = e.data["status"].as_str().unwrap_or("finished").into();
                        job.output = e.data.get("output").cloned().filter(|v| !v.is_null());
                        job.error = e.data.get("error").cloned().filter(|v| !v.is_null());
                    }
                }
                _ => {}
            }
        }
        Ok(jobs)
    }
    async fn cancel_job(&self, r: &Runtime, session: &str, id: &str) -> AppResult<Value> {
        self.ready.get_or_try_init(|| self.recover_all()).await?;
        let status = self.job_status(r, session, id).await?;
        if let Some((owner, token)) = self.jobs.lock().unwrap().get(id)
            && owner == session
        {
            token.cancel();
        }
        r.append(session, "job.cancel_requested", json!({"id":id}))
            .await?;
        Ok(json!({"id":id,"status":status["status"],"cancelRequested":true}))
    }
    async fn delete_schedule(&self, r: &Runtime, session: &str, id: &str) -> AppResult<()> {
        let _lock = self.schedules_lock.lock().await;
        let mut schedules = self.schedules().await?;
        let position = schedules
            .iter()
            .position(|s| s.id == id && s.session_id == session)
            .ok_or_else(|| AppError::new("SCHEDULE_SCOPE", "任务不存在或不属于本会话"))?;
        schedules.remove(position);
        self.save(&schedules).await?;
        r.append(session, "schedule.deleted", json!({"id":id}))
            .await?;
        Ok(())
    }
    async fn recover_jobs(&self) -> AppResult<()> {
        let Some(r) = self.runtime.upgrade() else {
            return Ok(());
        };
        for s in r.store.sessions(None).await? {
            let mut open = HashMap::new();
            for e in r.store.events(&s.id, 0).await? {
                if let Some(id) = e.data["id"].as_str() {
                    match e.kind.as_str() {
                        "job.started" => {
                            open.insert(id.to_owned(), ());
                        }
                        "job.finished" => {
                            open.remove(id);
                        }
                        _ => {}
                    }
                }
            }
            for id in open.keys() {
                if !self.jobs.lock().unwrap().contains_key(id) {
                    r.append(&s.id,"job.finished",json!({"id":id,"status":"interrupted","error":"应用中断，未重放Shell副作用"})).await?;
                }
            }
        }
        Ok(())
    }
    /// First readiness step: settle both jobs and terminals left open by a restart.
    async fn recover_all(&self) -> AppResult<()> {
        self.recover_jobs().await?;
        self.recover_terminals().await
    }
    /// Opened terminals without a `terminal.closed` event are settled as interrupted;
    /// side effects are never replayed.
    async fn recover_terminals(&self) -> AppResult<()> {
        let Some(r) = self.runtime.upgrade() else {
            return Ok(());
        };
        for s in r.store.sessions(None).await? {
            let mut open = HashMap::new();
            for e in r.store.events(&s.id, 0).await? {
                if let Some(id) = e.data["id"].as_str() {
                    match e.kind.as_str() {
                        "terminal.opened" => {
                            open.insert(id.to_owned(), ());
                        }
                        "terminal.closed" => {
                            open.remove(id);
                        }
                        _ => {}
                    }
                }
            }
            for id in open.keys() {
                r.append(
                    &s.id,
                    "terminal.closed",
                    json!({"id":id,"status":"interrupted","error":"应用中断，未重放终端副作用"}),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn schedules(&self) -> AppResult<Vec<Schedule>> {
        let r = self
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))?;
        serde_json::from_value(r.store.get(SCHEDULE_KEY).await?.unwrap_or(json!([]))).map_err(error)
    }
    async fn save(&self, s: &[Schedule]) -> AppResult<()> {
        let r = self
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))?;
        r.store.set(SCHEDULE_KEY, &json!(s)).await
    }
    async fn automations(&self) -> AppResult<Vec<Automation>> {
        let r = self
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))?;
        serde_json::from_value(r.store.get(AUTOMATION_KEY).await?.unwrap_or(json!([])))
            .map_err(error)
    }
    async fn save_automations(&self, a: &[Automation]) -> AppResult<()> {
        let r = self
            .runtime
            .upgrade()
            .ok_or_else(|| AppError::new("SHUTDOWN", "应用已关闭"))?;
        r.store.set(AUTOMATION_KEY, &json!(a)).await
    }
    /// Bound target session must exist; unbound automations get a fresh session in their workspace.
    async fn automation_session(&self, r: &Runtime, a: &Automation) -> AppResult<String> {
        match a.session_id.as_deref() {
            Some(sid) => {
                r.store.session(sid).await?;
                Ok(sid.to_owned())
            }
            None => {
                let title = if a.title.trim().is_empty() {
                    "自动化任务".to_owned()
                } else {
                    a.title.clone()
                };
                Ok(r.store.create_session(&a.workspace_id, &title).await?.id)
            }
        }
    }
    /// Creation/update guard: a bound session must exist and belong to the automation's workspace.
    async fn check_automation_session(
        &self,
        r: &Runtime,
        session_id: Option<&str>,
        workspace: &str,
    ) -> AppResult<()> {
        let Some(sid) = session_id else {
            return Ok(());
        };
        let s = r.store.session(sid).await?;
        if s.workspace_id != workspace {
            return Err(AppError::new(
                "AUTOMATION_SESSION",
                "绑定的会话必须属于当前工作区",
            ));
        }
        Ok(())
    }
    /// Run one dispatched automation turn in the target session, then settle its run record.
    async fn dispatch_automation(
        self: Arc<Self>,
        r: Arc<Runtime>,
        automation_id: String,
        run_id: String,
        session_id: String,
        provider: Arc<dyn ModelProvider>,
        stop: CancellationToken,
    ) {
        let (title, prompt) = {
            let _lock = self.schedules_lock.lock().await;
            match self.automations().await.ok().and_then(|all| {
                all.iter()
                    .find(|x| x.id == automation_id)
                    .map(|a| (a.title.clone(), a.prompt.clone()))
            }) {
                Some(v) => v,
                None => return,
            }
        };
        let _ = r
            .append(
                &session_id,
                "automation.triggered",
                json!({"id":automation_id,"runId":run_id,"title":title}),
            )
            .await;
        let result = run_owned(r.clone(), session_id.clone(), prompt, provider, stop).await;
        let _ = self
            .settle_automation_run(&r, &automation_id, &run_id, &session_id, result)
            .await;
    }
    /// Write back the terminal state of one run; once-kind automations end, periodic ones continue.
    async fn settle_automation_run(
        &self,
        r: &Runtime,
        automation_id: &str,
        run_id: &str,
        session_id: &str,
        result: AppResult<()>,
    ) -> AppResult<()> {
        let _lock = self.schedules_lock.lock().await;
        let mut all = self.automations().await?;
        let Some(a) = all.iter_mut().find(|x| x.id == automation_id) else {
            return Ok(());
        };
        let (status, err) = match &result {
            Ok(()) => ("succeeded", None),
            Err(e) => ("failed", Some(e.to_string())),
        };
        if let Some(run) = a.runs.iter_mut().find(|x| x.id == run_id) {
            run.status = status.into();
            run.error = err.clone();
        }
        a.status = match (a.kind, result.is_ok()) {
            (AutomationKind::Once, true) => "completed".into(),
            (AutomationKind::Once, false) => "failed".into(),
            _ => "active".into(),
        };
        let _ = r
            .append(
                session_id,
                "automation.finished",
                json!({"id":automation_id,"runId":run_id,"status":status,"error":err}),
            )
            .await;
        self.save_automations(&all).await
    }
    /// Scheduler pass over automations; mirrors [`Self::tick`] with catch-up never replayed.
    async fn tick_automations(self: &Arc<Self>, recover: bool) -> AppResult<()> {
        let _lock = self.schedules_lock.lock().await;
        let Some(r) = self.runtime.upgrade() else {
            return Ok(());
        };
        let mut all = self.automations().await?;
        let mut dispatch = Vec::new();
        for a in &mut all {
            if recover && a.status == "running" {
                a.status = "interrupted".into();
                if let Some(sid) = &a.session_id {
                    let _ = r
                        .append(sid, "automation.interrupted", json!({"id":a.id}))
                        .await;
                }
                continue;
            }
            if !a.enabled || a.status != "active" {
                continue;
            }
            let due = a
                .next_run_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc));
            let Some(next) = due else {
                // Missing next run (e.g. legacy data): recompute instead of firing blindly.
                a.next_run_at =
                    automation_next_run_at(&Local, a, Local::now()).map(|t| t.to_rfc3339());
                continue;
            };
            if next > Utc::now() {
                continue;
            }
            if recover {
                // Missed runs are never replayed; one-shot automations end as missed.
                if a.kind == AutomationKind::Once {
                    a.status = "missed".into();
                    if let Some(sid) = &a.session_id {
                        let _ = r.append(sid, "automation.missed", json!({"id":a.id})).await;
                    }
                } else {
                    a.next_run_at =
                        automation_next_run_at(&Local, a, Local::now()).map(|t| t.to_rfc3339());
                }
                continue;
            }
            let scheduled_at = a.next_run_at.clone();
            // Persist the claim before dispatch so a restart never repeats an ambiguous side effect.
            if a.kind == AutomationKind::Once {
                a.status = "running".into();
            } else {
                a.next_run_at =
                    automation_next_run_at(&Local, a, Local::now()).map(|t| t.to_rfc3339());
            }
            let provider = match automation_provider_of(&r).await {
                Err(e) => {
                    let record_run = a.kind == AutomationKind::Once;
                    if record_run {
                        a.status = "failed".into();
                    }
                    a.runs.insert(
                        0,
                        AutomationRun {
                            id: new_id(),
                            scheduled_at,
                            triggered_at: now(),
                            trigger: "schedule".into(),
                            status: "failed".into(),
                            session_id: a.session_id.clone().unwrap_or_default(),
                            error: Some(e.to_string()),
                        },
                    );
                    a.runs.truncate(AUTOMATION_RUNS_KEEP);
                    continue;
                }
                Ok(p) => p,
            };
            match self.automation_session(&r, a).await {
                Err(e) => {
                    if a.kind == AutomationKind::Once {
                        a.status = "failed".into();
                    }
                    a.runs.insert(
                        0,
                        AutomationRun {
                            id: new_id(),
                            scheduled_at,
                            triggered_at: now(),
                            trigger: "schedule".into(),
                            status: "failed".into(),
                            session_id: a.session_id.clone().unwrap_or_default(),
                            error: Some(e.to_string()),
                        },
                    );
                    a.runs.truncate(AUTOMATION_RUNS_KEEP);
                }
                Ok(session_id) => {
                    let run = AutomationRun {
                        id: new_id(),
                        scheduled_at,
                        triggered_at: now(),
                        trigger: "schedule".into(),
                        status: "running".into(),
                        session_id: session_id.clone(),
                        error: None,
                    };
                    let run_id = run.id.clone();
                    a.runs.insert(0, run);
                    a.runs.truncate(AUTOMATION_RUNS_KEEP);
                    a.run_count += 1;
                    a.last_run_at = Some(now());
                    dispatch.push((a.id.clone(), run_id, session_id, provider));
                }
            }
        }
        self.save_automations(&all).await?;
        for (automation_id, run_id, session_id, provider) in dispatch {
            let state = self.clone();
            let rt = r.clone();
            let stop = self.stop.child_token();
            tokio::spawn(async move {
                state
                    .dispatch_automation(rt, automation_id, run_id, session_id, provider, stop)
                    .await;
            });
        }
        Ok(())
    }
    async fn tick(self: &Arc<Self>, recover: bool) -> AppResult<()> {
        let _lock = self.schedules_lock.lock().await;
        let Some(r) = self.runtime.upgrade() else {
            return Ok(());
        };
        let mut schedules = self.schedules().await?;
        let mut dispatch = Vec::new();
        for s in &mut schedules {
            if recover && s.status == "running" {
                s.status = "interrupted".into();
                self.save_single_event(&r, s).await?;
                continue;
            }
            if s.status != "pending" {
                continue;
            }
            let due = DateTime::parse_from_rfc3339(&s.at).map_err(error)? <= Utc::now();
            if !due {
                continue;
            }
            if recover {
                s.status = "missed".into();
                self.save_single_event(&r, s).await?;
                continue;
            }
            let provider = r.models.build_model(&s.model_id).await;
            match provider {
                Err(e) => {
                    s.status = "failed".into();
                    r.append(
                        &s.session_id,
                        "schedule.failed",
                        json!({"id":s.id,"error":e}),
                    )
                    .await?;
                }
                Ok(provider) => {
                    s.status = "running".into();
                    self.save_single_event(&r, s).await?;
                    // Persist running before dispatch so restart never repeats an ambiguous side effect.
                    dispatch.push((s.clone(), provider));
                }
            }
        }
        self.save(&schedules).await?;
        for (schedule, provider) in dispatch {
            let state = self.clone();
            let runtime = r.clone();
            let stop = self.stop.child_token();
            *self
                .agents
                .lock()
                .unwrap()
                .entry(schedule.session_id.clone())
                .or_insert(0) += 1;
            tokio::spawn(async move {
                let result = run_owned(
                    runtime.clone(),
                    schedule.session_id.clone(),
                    schedule.task,
                    provider,
                    stop,
                )
                .await;
                {
                    let mut agents = state.agents.lock().unwrap();
                    if let Some(n) = agents.get_mut(&schedule.session_id) {
                        *n -= 1;
                        if *n == 0 {
                            agents.remove(&schedule.session_id);
                        }
                    }
                }
                let _guard = state.schedules_lock.lock().await;
                if let Ok(mut all) = state.schedules().await
                    && let Some(current) = all.iter_mut().find(|x| x.id == schedule.id)
                {
                    current.status = if result.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                    .into();
                    let _ = runtime
                        .append(
                            &current.session_id,
                            "schedule.finished",
                            json!({"id":current.id,"status":current.status,"error":result.err()}),
                        )
                        .await;
                    let _ = state.save(&all).await;
                }
            });
        }
        Ok(())
    }
    async fn save_single_event(&self, r: &Runtime, s: &Schedule) -> AppResult<()> {
        r.append(
            &s.session_id,
            &format!("schedule.{}", s.status),
            json!({"id":s.id,"status":s.status}),
        )
        .await?;
        Ok(())
    }
}
async fn run_owned(
    runtime: Arc<Runtime>,
    session: String,
    task: String,
    provider: Arc<dyn ModelProvider>,
    stop: CancellationToken,
) -> AppResult<()> {
    if stop.is_cancelled() {
        return Err(harness_runtime::model::cancelled());
    }
    let run = runtime.clone().run_session(session.clone(), task, provider);
    tokio::pin!(run);
    let mut cancelled = false;
    loop {
        tokio::select! {
         result=&mut run=>return result,
         _=stop.cancelled(),if !cancelled=>{cancelled=true;runtime.cancel(&session);},
         _=tokio::time::sleep(Duration::from_millis(10)),if cancelled=>{runtime.cancel(&session);}
        }
    }
}
/// Human command only: serializes with turns and retains every raw event.
pub async fn compact_session(
    runtime: Arc<Runtime>,
    session: &str,
    provider: Arc<dyn ModelProvider>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let _boundary = runtime.session_boundary(session).await;
    runtime.store.session(session).await?;
    let events = runtime.store.events(session, 0).await?;
    let mut messages = harness_runtime::agent::project_messages(&events);
    for message in &mut messages {
        message.replay = None;
        message.reasoning.clear();
    }
    let request = vec![
        ModelMessage::system(
            "请用中文总结会话目标、已完成工作、关键事实、未完成事项和工具执行结果。保留继续任务所需信息，不编造。",
        ),
        ModelMessage::user(serde_json::to_string(&messages).map_err(error)?),
    ];
    let answer = provider
        .complete(request, vec![], cancel.clone(), Arc::new(|_| {}))
        .await?;
    if cancel.is_cancelled() {
        return Err(harness_runtime::model::cancelled());
    }
    if answer.content.trim().is_empty() || !answer.calls.is_empty() {
        return Err(AppError::new(
            "COMPACTION_INVALID",
            "模型没有返回有效纯文本摘要",
        ));
    }
    runtime
        .append(
            session,
            "compaction.summary",
            json!({"content":answer.content,"throughSeq":events.last().map(|e|e.seq.clone())}),
        )
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests;
