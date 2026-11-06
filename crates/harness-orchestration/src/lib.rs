//! Durable jobs, scoped agent messaging and application-lifetime scheduling.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const SCHEDULE_KEY: &str = "orchestration.schedules.v1";
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
}
impl Drop for Installed {
    fn drop(&mut self) {
        self.state.stop.cancel();
        for (_, token) in self.state.jobs.lock().unwrap().values() {
            token.cancel();
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
    let state = Arc::new(State {
        runtime: Arc::downgrade(&runtime),
        stop: CancellationToken::new(),
        jobs: Mutex::new(HashMap::new()),
        agents: Mutex::new(HashMap::new()),
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
            .get_or_try_init(|| ticker.recover_jobs())
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
            "background_status" | "agent_wait" | "schedule_list"
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
            _ => return Err(AppError::new("TOOL_UNAVAILABLE", &self.name)),
        };
        Ok(ToolResult {
            content: value.to_string(),
            is_error: false,
        })
    }
}
impl State {
    async fn job_status(&self, r: &Runtime, session: &str, id: &str) -> AppResult<Value> {
        self.ready.get_or_try_init(|| self.recover_jobs()).await?;
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
        self.ready.get_or_try_init(|| self.recover_jobs()).await?;
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
        self.ready.get_or_try_init(|| self.recover_jobs()).await?;
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
