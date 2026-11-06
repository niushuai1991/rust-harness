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
