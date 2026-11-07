use super::*;
use harness_runtime::model::{DeltaSink, ModelAnswer};
struct Answer;
#[async_trait::async_trait]
impl ModelProvider for Answer {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        Ok(ModelAnswer {
            content: "摘要和回复".into(),
            ..Default::default()
        })
    }
}
async fn setup() -> (tempfile::TempDir, Arc<Runtime>, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("data")).await.unwrap();
    let workspace = runtime.workspaces.create(&source).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "父任务")
        .await
        .unwrap();
    let ctx = ToolContext {
        session_id: session.id,
        workspace_id: workspace.id,
        root: workspace.replica_path.into(),
        provider: Arc::new(Answer),
    };
    (dir, runtime, ctx)
}
async fn call(i: &Installed, c: &ToolContext, name: &str, args: Value) -> AppResult<Value> {
    let t = Tool {
        state: i.state.clone(),
        name: name.into(),
    };
    let r = t.execute(c.clone(), args, CancellationToken::new()).await?;
    serde_json::from_str(&r.content).map_err(error)
}
#[tokio::test]
async fn durable_schedules_delete_and_scope() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let s = call(
        &i,
        &c,
        "schedule_write",
        json!({"at":"2099-01-01T00:00:00Z","task":"以后执行","modelId":"missing"}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&i, &c, "schedule_list", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    call(&i, &c, "schedule_delete", json!({"id":s["id"]}))
        .await
        .unwrap();
    assert!(
        call(&i, &c, "schedule_list", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(
            &i,
            &c,
            "agent_send",
            json!({"sessionId":"outside","task":"越权"})
        )
        .await
        .unwrap_err()
        .code,
        "AGENT_SCOPE"
    );
    assert_eq!(
        call(&i, &c, "agent_wait", json!({"sessionId":"outside"}))
            .await
            .unwrap_err()
            .code,
        "AGENT_SCOPE"
    );
    assert_eq!(
        call(&i, &c, "agent_fork", json!({"sessionId":"outside"}))
            .await
            .unwrap_err()
            .code,
        "AGENT_SCOPE"
    );
}
#[tokio::test]
async fn compaction_preserves_raw_events() {
    let (_d, r, c) = setup().await;
    r.append(&c.session_id, "user.message", json!({"content":"原始消息"}))
        .await
        .unwrap();
    compact_session(
        r.clone(),
        &c.session_id,
        c.provider.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let e = r.store.events(&c.session_id, 0).await.unwrap();
    assert_eq!(e[0].data["content"], "原始消息");
    assert_eq!(e.last().unwrap().kind, "compaction.summary");
    assert_eq!(
        harness_runtime::agent::project_messages(&e)[1].content,
        "摘要和回复"
    );
}
#[tokio::test]
async fn scoped_fork_send_wait() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let f = call(&i, &c, "agent_fork", json!({"sessionId":c.session_id}))
        .await
        .unwrap();
    let child = f["sessionId"].as_str().unwrap();
    call(
        &i,
        &c,
        "agent_send",
        json!({"sessionId":child,"task":"真实子任务"}),
    )
    .await
    .unwrap();
    let done = call(
        &i,
        &c,
        "agent_wait",
        json!({"sessionId":child,"timeoutMs":5000}),
    )
    .await
    .unwrap();
    assert_eq!(done["status"], "completed");
    assert!(
        r.store
            .events(child, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "assistant.message")
    );
}
#[tokio::test]
async fn overdue_recovery_marks_missed_and_future_missing_model_fails() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    // This test drives recovery/ticks manually; the background first tick must not race it.
    i.state.stop.cancel();
    let s = Schedule {
        id: new_id(),
        session_id: c.session_id.clone(),
        workspace_id: c.workspace_id.clone(),
        at: "2000-01-01T00:00:00Z".into(),
        task: "不能重放".into(),
        model_id: "missing".into(),
        status: "pending".into(),
    };
    r.store.set(SCHEDULE_KEY, &json!([s])).await.unwrap();
    i.state.tick(true).await.unwrap();
    assert_eq!(i.state.schedules().await.unwrap()[0].status, "missed");
    let mut s = i.state.schedules().await.unwrap();
    s[0].status = "pending".into();
    i.state.save(&s).await.unwrap();
    i.state.tick(false).await.unwrap();
    assert_eq!(i.state.schedules().await.unwrap()[0].status, "failed");
}
#[cfg(windows)]
#[tokio::test]
async fn background_shell_persists_result_and_cancellation() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let j = call(
        &i,
        &c,
        "background_run",
        json!({"command":"Write-Output hello"}),
    )
    .await
    .unwrap();
    let id = j["id"].as_str().unwrap();
    for _ in 0..100 {
        let s = call(&i, &c, "background_status", json!({"id":id}))
            .await
            .unwrap();
        if s["status"] != "running" {
            assert_eq!(s["status"], "completed");
            assert!(s["output"]["stdout"].as_str().unwrap().contains("hello"));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let j = call(
        &i,
        &c,
        "background_run",
        json!({"command":"Start-Sleep -Seconds 30"}),
    )
    .await
    .unwrap();
    call(&i, &c, "background_cancel", json!({"id":j["id"]}))
        .await
        .unwrap();
    for _ in 0..100 {
        let s = call(&i, &c, "background_status", json!({"id":j["id"]}))
            .await
            .unwrap();
        if s["status"] != "running" {
            assert_eq!(s["status"], "cancelled");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("后台取消未结算");
}
#[tokio::test]
async fn recovery_never_replays_jobs_and_status_is_session_scoped() {
    let (_d, r, c) = setup().await;
    r.append(
        &c.session_id,
        "job.started",
        json!({"id":"old","command":"throw unsafe","status":"running"}),
    )
    .await
    .unwrap();
    let i = install(r.clone()).unwrap();
    let status = call(&i, &c, "background_status", json!({"id":"old"}))
        .await
        .unwrap();
    assert_eq!(status["status"], "interrupted");
    let other = r
        .store
        .create_session(&c.workspace_id, "另一会话")
        .await
        .unwrap();
    let mut other_context = c.clone();
    other_context.session_id = other.id;
    assert_eq!(
        call(&i, &other_context, "background_status", json!({"id":"old"}))
            .await
            .unwrap_err()
            .code,
        "JOB_SCOPE"
    );
}
#[cfg(windows)]
#[tokio::test]
async fn dropping_install_cancels_shell_before_releasing_workspace_lease() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let j = call(
        &i,
        &c,
        "background_run",
        json!({"command":"Start-Sleep -Seconds 30"}),
    )
    .await
    .unwrap();
    assert_eq!(
        r.workspaces.refresh(&c.workspace_id).unwrap_err().code,
        "BUSY"
    );
    drop(i);
    for _ in 0..200 {
        if let Some(e) = r
            .store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .rev()
            .find(|e| e.kind == "job.finished" && e.data["id"] == j["id"])
        {
            assert_eq!(e.data["status"], "cancelled");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("关闭后任务未取消");
}
#[tokio::test]
async fn session_jobs_reports_merged_status_and_session_scope() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    r.append(
        &c.session_id,
        "job.started",
        json!({"id":"old","command":"echo gone","status":"running"}),
    )
    .await
    .unwrap();
    r.append(&c.session_id, "job.cancel_requested", json!({"id":"old"}))
        .await
        .unwrap();
    r.append(
        &c.session_id,
        "job.finished",
        json!({"id":"old","status":"interrupted","error":"应用中断，未重放Shell副作用"}),
    )
    .await
    .unwrap();
    let jobs = i.session_jobs(&c.session_id).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "old");
    assert_eq!(jobs[0].command, "echo gone");
    assert_eq!(jobs[0].status, "interrupted");
    assert!(jobs[0].cancel_requested);
    assert_eq!(
        jobs[0].error.as_ref().unwrap(),
        "应用中断，未重放Shell副作用"
    );
    let other = r
        .store
        .create_session(&c.workspace_id, "另一会话")
        .await
        .unwrap();
    assert!(i.session_jobs(&other.id).await.unwrap().is_empty());
}
#[cfg(windows)]
#[tokio::test]
async fn cancel_job_scopes_and_marks_cancellation() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let j = call(
        &i,
        &c,
        "background_run",
        json!({"command":"Start-Sleep -Seconds 30"}),
    )
    .await
    .unwrap();
    let id = j["id"].as_str().unwrap().to_owned();
    let other = r
        .store
        .create_session(&c.workspace_id, "另一会话")
        .await
        .unwrap();
    assert_eq!(
        i.cancel_job(&other.id, &id).await.unwrap_err().code,
        "JOB_SCOPE"
    );
    i.cancel_job(&c.session_id, &id).await.unwrap();
    for _ in 0..100 {
        let jobs = i.session_jobs(&c.session_id).await.unwrap();
        let job = jobs.iter().find(|j| j.id == id).unwrap();
        if job.status != "running" {
            assert_eq!(job.status, "cancelled");
            assert!(job.cancel_requested);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("人工取消未结算");
}
#[tokio::test]
async fn schedule_host_api_scopes_and_deletes() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    let s = call(
        &i,
        &c,
        "schedule_write",
        json!({"at":"2099-01-01T00:00:00Z","task":"面板删除","modelId":"missing"}),
    )
    .await
    .unwrap();
    let other = r
        .store
        .create_session(&c.workspace_id, "另一会话")
        .await
        .unwrap();
    assert!(i.session_schedules(&other.id).await.unwrap().is_empty());
    let mine = i.session_schedules(&c.session_id).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, s["id"]);
    assert_eq!(mine[0].task, "面板删除");
    assert_eq!(mine[0].status, "pending");
    i.delete_schedule(&c.session_id, &mine[0].id).await.unwrap();
    assert!(i.session_schedules(&c.session_id).await.unwrap().is_empty());
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "schedule.deleted")
    );
    assert_eq!(
        i.delete_schedule(&c.session_id, &mine[0].id)
            .await
            .unwrap_err()
            .code,
        "SCHEDULE_SCOPE"
    );
}
struct StubWorld;
#[async_trait::async_trait]
impl PtyWorld for StubWorld {
    async fn spawn(
        &self,
        _root: &std::path::Path,
        _cancel: CancellationToken,
    ) -> AppResult<Box<dyn TerminalSession>> {
        Ok(Box::new(StubTerminal {
            echo: String::new(),
            exited: false,
        }))
    }
}
struct StubTerminal {
    echo: String,
    exited: bool,
}
#[async_trait::async_trait]
impl TerminalSession for StubTerminal {
    async fn write_input(&mut self, input: &str) -> AppResult<()> {
        if self.exited {
            return Err(AppError::new("TERMINAL_GONE", "终端已退出"));
        }
        self.echo.push_str(input);
        if input.contains("exit") {
            self.exited = true;
        }
        Ok(())
    }
    async fn read_output(&mut self, _yield_ms: u64) -> AppResult<String> {
        Ok(format!("回显:{}", std::mem::take(&mut self.echo)))
    }
    async fn exit_code(&mut self) -> Option<i32> {
        self.exited.then_some(0)
    }
    fn spill(&self) -> (String, bool) {
        ("stub 尾部输出".into(), false)
    }
    fn terminate(&self) {}
}
#[tokio::test]
async fn terminal_open_write_read_close_with_stub_world() {
    let (_d, r, c) = setup().await;
    let i = install_with(r.clone(), Arc::new(StubWorld)).unwrap();
    let opened = call(&i, &c, "terminal_open", json!({})).await.unwrap();
    let id = opened["id"].as_str().unwrap().to_owned();
    // 共享租约持有：终端存续期间人工同步（独占 refresh）被拒
    assert_eq!(
        r.workspaces.refresh(&c.workspace_id).unwrap_err().code,
        "BUSY"
    );
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "terminal.opened" && e.data["id"] == opened["id"])
    );
    let written = call(
        &i,
        &c,
        "terminal_write",
        json!({"id":id,"input":"hello","yieldMs":100}),
    )
    .await
    .unwrap();
    assert_eq!(written["status"], "running");
    assert_eq!(written["output"], "回显:hello");
    let read = call(&i, &c, "terminal_read", json!({"id":id,"yieldMs":50}))
        .await
        .unwrap();
    assert_eq!(read["status"], "running");
    let closed = call(&i, &c, "terminal_close", json!({"id":id}))
        .await
        .unwrap();
    assert_eq!(closed["closed"], true);
    let done = r
        .store
        .events(&c.session_id, 0)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.kind == "terminal.closed" && e.data["id"] == opened["id"])
        .expect("缺少 terminal.closed 事件");
    assert_eq!(done.data["status"], "terminated");
    assert!(done.data["outputBlobHash"].is_string());
    r.workspaces.refresh(&c.workspace_id).unwrap();
    assert_eq!(
        call(&i, &c, "terminal_write", json!({"id":id,"input":"x"}))
            .await
            .unwrap_err()
            .code,
        "TERMINAL_SCOPE"
    );
}
#[tokio::test]
async fn terminal_exit_settles_and_scopes() {
    let (_d, r, c) = setup().await;
    let i = install_with(r.clone(), Arc::new(StubWorld)).unwrap();
    let opened = call(&i, &c, "terminal_open", json!({})).await.unwrap();
    let id = opened["id"].as_str().unwrap().to_owned();
    let written = call(
        &i,
        &c,
        "terminal_write",
        json!({"id":id,"input":"exit","yieldMs":100}),
    )
    .await
    .unwrap();
    assert_eq!(written["status"], "exited");
    assert_eq!(written["exitCode"], 0);
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "terminal.closed"
                && e.data["id"] == opened["id"]
                && e.data["status"] == "exited")
    );
    let other = r
        .store
        .create_session(&c.workspace_id, "另一会话")
        .await
        .unwrap();
    let mut other_context = c.clone();
    other_context.session_id = other.id;
    assert_eq!(
        call(&i, &other_context, "terminal_read", json!({"id":id}))
            .await
            .unwrap_err()
            .code,
        "TERMINAL_SCOPE"
    );
}
#[tokio::test]
async fn terminal_limit_approval_and_recovery() {
    let (_d, r, c) = setup().await;
    r.append(
        &c.session_id,
        "terminal.opened",
        json!({"id":"old","shell":"pwsh","status":"running"}),
    )
    .await
    .unwrap();
    let i = install_with(r.clone(), Arc::new(StubWorld)).unwrap();
    for _ in 0..200 {
        let settled = r
            .store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| {
                e.kind == "terminal.closed"
                    && e.data["id"] == "old"
                    && e.data["status"] == "interrupted"
            });
        if settled {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "terminal.closed"
                && e.data["id"] == "old"
                && e.data["status"] == "interrupted"),
        "重启后未按 interrupted 结算未关闭终端"
    );
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(call(&i, &c, "terminal_open", json!({})).await.unwrap()["id"].clone());
    }
    assert_eq!(
        call(&i, &c, "terminal_open", json!({}))
            .await
            .unwrap_err()
            .code,
        "TERMINAL_LIMIT"
    );
    let approval_of = |name: &str| {
        Tool {
            state: i.state.clone(),
            name: name.into(),
        }
        .requires_approval()
    };
    assert!(approval_of("terminal_open"));
    assert!(!approval_of("terminal_write"));
    assert!(!approval_of("terminal_read"));
    assert!(!approval_of("terminal_close"));
    for id in ids {
        call(&i, &c, "terminal_close", json!({"id":id}))
            .await
            .unwrap();
    }
    r.workspaces.refresh(&c.workspace_id).unwrap();
}

// ---------- 自动化任务 ----------

use chrono::{Datelike, FixedOffset, TimeZone};

fn automation_tz() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}
fn automation_local(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> DateTime<FixedOffset> {
    automation_tz()
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .unwrap()
}
fn automation_draft(kind: AutomationKind) -> Automation {
    Automation {
        id: new_id(),
        title: "演示任务".into(),
        prompt: "到期要执行的内容".into(),
        session_id: None,
        workspace_id: "ws".into(),
        kind,
        at: None,
        time: Some("09:30".into()),
        weekdays: None,
        month_day: None,
        month: None,
        enabled: true,
        status: "active".into(),
        next_run_at: None,
        last_run_at: None,
        run_count: 0,
        runs: Vec::new(),
        created_at: now(),
        updated_at: now(),
    }
}
fn automation_utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> String {
    automation_local(y, m, d, h, mi)
        .with_timezone(&Utc)
        .to_rfc3339()
}

#[tokio::test]
async fn automation_next_run_at_table() {
    let t = automation_tz();
    let build = |kind, patch: &dyn Fn(&mut Automation)| {
        let mut a = automation_draft(kind);
        patch(&mut a);
        a
    };
    // daily：当天未过 → 当天；已过 → 明天；缺 time → None
    let daily = |time: &str| build(AutomationKind::Daily, &|a| a.time = Some(time.into()));
    assert_eq!(
        automation_next_run_at(&t, &daily("09:30"), automation_local(2026, 11, 7, 8, 0)),
        Some(automation_local(2026, 11, 7, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &daily("09:30"), automation_local(2026, 11, 7, 9, 30)),
        Some(automation_local(2026, 11, 8, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &daily(""), automation_local(2026, 11, 7, 8, 0)),
        None
    );
    // weekly：多日命中最近的；当天命中且未过 → 当天；已过 → 下一个命中日；空 weekdays → None
    let weekly = |days: &[u8]| {
        build(AutomationKind::Weekly, &|a| {
            a.weekdays = Some(days.to_vec());
        })
    };
    assert_eq!(
        automation_next_run_at(&t, &weekly(&[1, 5]), automation_local(2026, 11, 7, 20, 0)),
        Some(automation_local(2026, 11, 9, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &weekly(&[1, 5]), automation_local(2026, 11, 9, 10, 0)),
        Some(automation_local(2026, 11, 13, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &weekly(&[6]), automation_local(2026, 11, 7, 8, 0)),
        Some(automation_local(2026, 11, 7, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &weekly(&[]), automation_local(2026, 11, 7, 8, 0)),
        None
    );
    // monthly：31 日遇 11 月 → 12 月 31 日；1 日已过 → 下月 1 日；非法日 → None
    let monthly = |day: u8| {
        build(AutomationKind::Monthly, &|a| {
            a.month_day = Some(day);
        })
    };
    assert_eq!(
        automation_next_run_at(&t, &monthly(31), automation_local(2026, 11, 7, 8, 0)),
        Some(automation_local(2026, 12, 31, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &monthly(1), automation_local(2026, 11, 7, 8, 0)),
        Some(automation_local(2026, 12, 1, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &monthly(0), automation_local(2026, 11, 7, 8, 0)),
        None
    );
    // yearly：平年无 2/29 → 跳到 2028；当天未过 → 当天；已过 → 明年
    let yearly = |month: u8, day: u8| {
        build(AutomationKind::Yearly, &|a| {
            a.month = Some(month);
            a.month_day = Some(day);
        })
    };
    assert_eq!(
        automation_next_run_at(&t, &yearly(2, 29), automation_local(2026, 11, 7, 8, 0)),
        Some(automation_local(2028, 2, 29, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &yearly(12, 31), automation_local(2026, 12, 31, 8, 0)),
        Some(automation_local(2026, 12, 31, 9, 30).with_timezone(&Utc))
    );
    assert_eq!(
        automation_next_run_at(&t, &yearly(12, 31), automation_local(2026, 12, 31, 22, 0)),
        Some(automation_local(2027, 12, 31, 9, 30).with_timezone(&Utc))
    );
    // once：返回 at 本身（含时区归一化为 UTC）
    let once = build(AutomationKind::Once, &|a| {
        a.at = Some("2099-01-01T00:00:00+08:00".into());
    });
    assert_eq!(
        automation_next_run_at(&t, &once, automation_local(2026, 11, 7, 8, 0)),
        Some(
            DateTime::parse_from_rfc3339("2099-01-01T00:00:00+08:00")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
    assert_eq!(
        automation_utc(2026, 11, 7, 9, 30),
        DateTime::parse_from_rfc3339("2026-11-07T09:30:00+08:00")
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339()
    );
}

fn automation_params(ws: &str, kind: AutomationKind) -> AutomationParams {
    AutomationParams {
        workspace_id: ws.into(),
        title: Some("演示任务".into()),
        prompt: "到期要执行的内容".into(),
        session_id: None,
        kind,
        at: None,
        time: Some("09:30".into()),
        weekdays: None,
        month_day: None,
        month: None,
    }
}

#[tokio::test]
async fn automation_lifecycle_validates_and_updates() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    // prompt 为空被拒绝
    let mut p = automation_params(&c.workspace_id, AutomationKind::Daily);
    p.prompt = "  ".into();
    assert_eq!(
        i.automation_create(p).await.unwrap_err().code,
        "AUTOMATION_ARGUMENTS"
    );
    // once 缺时间 / 时刻在过去被拒绝
    let mut once = automation_params(&c.workspace_id, AutomationKind::Once);
    once.time = None;
    assert_eq!(
        i.automation_create(once).await.unwrap_err().code,
        "AUTOMATION_ARGUMENTS"
    );
    let mut stale = automation_params(&c.workspace_id, AutomationKind::Once);
    stale.at = Some("2000-01-01T00:00:00Z".into());
    assert_eq!(
        i.automation_create(stale).await.unwrap_err().code,
        "AUTOMATION_TIME"
    );
    // weekly 空 weekdays 被拒绝
    let mut weekly = automation_params(&c.workspace_id, AutomationKind::Weekly);
    weekly.weekdays = Some(vec![]);
    assert_eq!(
        i.automation_create(weekly).await.unwrap_err().code,
        "AUTOMATION_ARGUMENTS"
    );
    // 绑定不属于本工作区的会话被拒绝
    let other = r.store.create_session("other-ws", "别处").await.unwrap();
    let mut bound = automation_params(&c.workspace_id, AutomationKind::Daily);
    bound.session_id = Some(other.id);
    assert_eq!(
        i.automation_create(bound).await.unwrap_err().code,
        "AUTOMATION_SESSION"
    );
    // 创建成功：active、next_run_at 与纯函数一致
    let a = i
        .automation_create(automation_params(&c.workspace_id, AutomationKind::Daily))
        .await
        .unwrap();
    assert_eq!(a.status, "active");
    assert!(a.enabled);
    assert_eq!(
        a.next_run_at,
        automation_next_run_at(&Local, &a, Local::now()).map(|t| t.to_rfc3339())
    );
    // 编辑重算 next_run_at
    let mut changed = automation_params(&c.workspace_id, AutomationKind::Daily);
    changed.time = Some("22:00".into());
    let updated = i.automation_update(&a.id, changed).await.unwrap();
    assert_eq!(
        updated.next_run_at,
        automation_next_run_at(&Local, &updated, Local::now()).map(|t| t.to_rfc3339())
    );
    assert_ne!(updated.next_run_at, a.next_run_at);
    // 启停：暂停后恢复并重算过期触发点
    let paused = i.automation_set_enabled(&a.id, false).await.unwrap();
    assert_eq!(paused.status, "paused");
    assert!(!paused.enabled);
    let resumed = i.automation_set_enabled(&a.id, true).await.unwrap();
    assert_eq!(resumed.status, "active");
    assert_eq!(
        resumed.next_run_at,
        automation_next_run_at(&Local, &resumed, Local::now()).map(|t| t.to_rfc3339())
    );
    // 删除后再次删除 NOT_FOUND
    i.automation_delete(&a.id).await.unwrap();
    assert_eq!(
        i.automation_delete(&a.id).await.unwrap_err().code,
        "NOT_FOUND"
    );
}

#[tokio::test]
async fn automation_tick_and_recovery_semantics() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    // 手动驱动调度，避免后台首轮 tick 竞争。
    i.state.stop.cancel();
    // 强制模型解析失败，专注状态机分支。
    r.store
        .set(
            "app.default_selection",
            &json!({"modelId":"missing-model","effort":null}),
        )
        .await
        .unwrap();
    // 恢复：单次已过期 → missed；绑定会话收到 automation.missed 事件
    let mut once = automation_draft(AutomationKind::Once);
    once.at = Some("2000-01-01T00:00:00Z".into());
    once.next_run_at = Some(once.at.clone().unwrap());
    once.session_id = Some(c.session_id.clone());
    once.workspace_id = c.workspace_id.clone();
    i.state.save_automations(&[once]).await.unwrap();
    i.state.tick_automations(true).await.unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "missed");
    assert_eq!(after.runs.len(), 0);
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "automation.missed")
    );
    // 恢复：周期已过期 → 前推到未来，不补跑
    let mut daily = automation_draft(AutomationKind::Daily);
    daily.workspace_id = c.workspace_id.clone();
    daily.next_run_at = Some("2000-01-01T00:00:00Z".into());
    i.state.save_automations(&[daily]).await.unwrap();
    i.state.tick_automations(true).await.unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "active");
    assert_eq!(after.runs.len(), 0);
    let pushed = after.next_run_at.as_deref().unwrap();
    assert!(DateTime::parse_from_rfc3339(pushed).unwrap() > Utc::now());
    // 恢复：running → interrupted，且落事件
    let mut running = automation_draft(AutomationKind::Daily);
    running.workspace_id = c.workspace_id.clone();
    running.session_id = Some(c.session_id.clone());
    running.status = "running".into();
    i.state.save_automations(&[running]).await.unwrap();
    i.state.tick_automations(true).await.unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "interrupted");
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "automation.interrupted")
    );
    // 到期触发但模型不可用：单次 → failed 终态并记录失败 run
    let mut once_due = automation_draft(AutomationKind::Once);
    once_due.at = Some("2000-01-01T00:00:00Z".into());
    once_due.next_run_at = Some(Utc::now().to_rfc3339());
    once_due.workspace_id = c.workspace_id.clone();
    i.state.save_automations(&[once_due]).await.unwrap();
    i.state.tick_automations(false).await.unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "failed");
    assert_eq!(after.runs.len(), 1);
    assert_eq!(after.runs[0].status, "failed");
    assert!(after.runs[0].error.is_some());
    // 到期触发但模型不可用：周期 → 保持 active，记录失败 run，next_run_at 已前推
    let mut daily_due = automation_draft(AutomationKind::Daily);
    daily_due.workspace_id = c.workspace_id.clone();
    daily_due.next_run_at = Some(Utc::now().to_rfc3339());
    i.state.save_automations(&[daily_due]).await.unwrap();
    i.state.tick_automations(false).await.unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "active");
    assert_eq!(after.runs.len(), 1);
    assert_eq!(after.runs[0].status, "failed");
    assert!(
        DateTime::parse_from_rfc3339(after.next_run_at.as_deref().unwrap()).unwrap() > Utc::now()
    );
}

#[tokio::test]
async fn automation_settle_updates_run_and_status() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    i.state.stop.cancel();
    let run = |id: &str| AutomationRun {
        id: id.into(),
        scheduled_at: None,
        triggered_at: now(),
        trigger: "schedule".into(),
        status: "running".into(),
        session_id: c.session_id.clone(),
        error: None,
    };
    // 单次成功 → completed
    let mut once = automation_draft(AutomationKind::Once);
    once.session_id = Some(c.session_id.clone());
    once.workspace_id = c.workspace_id.clone();
    once.status = "running".into();
    once.run_count = 1;
    once.runs.push(run("run1"));
    i.state.save_automations(&[once]).await.unwrap();
    let once_id = i.state.automations().await.unwrap()[0].id.clone();
    i.state
        .settle_automation_run(&r, &once_id, "run1", &c.session_id, Ok(()))
        .await
        .unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "completed");
    assert_eq!(after.runs[0].status, "succeeded");
    assert!(
        r.store
            .events(&c.session_id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "automation.finished")
    );
    // 周期失败 → 保持 active，run 记录错误
    let mut daily = automation_draft(AutomationKind::Daily);
    daily.session_id = Some(c.session_id.clone());
    daily.workspace_id = c.workspace_id.clone();
    daily.run_count = 1;
    daily.runs.push(run("run2"));
    i.state.save_automations(&[daily]).await.unwrap();
    let daily_id = i.state.automations().await.unwrap()[0].id.clone();
    i.state
        .settle_automation_run(
            &r,
            &daily_id,
            "run2",
            &c.session_id,
            Err(AppError::new("X", "坏了")),
        )
        .await
        .unwrap();
    let after = &i.state.automations().await.unwrap()[0];
    assert_eq!(after.status, "active");
    assert_eq!(after.runs[0].status, "failed");
    assert_eq!(after.runs[0].error.as_deref(), Some("X: 坏了"));
}

#[tokio::test]
async fn automation_manual_run_fails_cleanly_without_model() {
    let (_d, r, c) = setup().await;
    let i = install(r.clone()).unwrap();
    i.state.stop.cancel();
    // 强制模型解析失败：手动执行应干净失败且不落 run。
    r.store
        .set(
            "app.default_selection",
            &json!({"modelId":"missing-model","effort":null}),
        )
        .await
        .unwrap();
    let a = i
        .automation_create(automation_params(&c.workspace_id, AutomationKind::Daily))
        .await
        .unwrap();
    // 无可用模型：手动执行返回错误且不落 run、不改计划时间
    let err = i.automation_run_now(&a.id).await.unwrap_err();
    assert!(
        [
            "AUTOMATION_MODEL",
            "MODEL_NOT_FOUND",
            "MODEL_DISABLED",
            "MODEL_CONFIG"
        ]
        .contains(&err.code.as_str())
    );
    let after = i.automation_list(&c.workspace_id).await.unwrap();
    assert_eq!(after[0].run_count, 0);
    assert!(after[0].runs.is_empty());
    assert_eq!(after[0].next_run_at, a.next_run_at);
}
