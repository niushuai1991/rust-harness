//! shell 前台到限自动转后台：任务继续执行并存续到回合结束、可经注册表取消、无孤儿进程；
//! 非 shell 工具到限维持「取消 + TOOL_TIMEOUT」现状。
#![cfg(windows)]
use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    ExternalTool, Runtime, ToolContext,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let mut answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}

struct Fixture {
    runtime: Arc<Runtime>,
    session: String,
    _dir: tempfile::TempDir,
    source: std::path::PathBuf,
}
/// 直接模式 fixture：shell 无沙箱宿主令牌执行，审批 Auto 放行，专注守卫与转后台行为。
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Direct)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    runtime.set_default_approval_policy(Some(ApprovalPolicy::Auto));
    runtime.set_tool_timeout_secs(5).await.unwrap();
    Fixture {
        runtime,
        session: session.id,
        _dir: dir,
        source,
    }
}

fn shell_call(id: &str, command: &str) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: json!({"command": command}),
        }],
        ..Default::default()
    }
}

async fn wait_event(
    rt: &Runtime,
    session: &str,
    pred: impl Fn(&SessionEvent) -> bool,
) -> SessionEvent {
    for _ in 0..600 {
        let events = rt.store.events(session, 0).await.unwrap();
        if let Some(e) = events.iter().rev().find(|e| pred(e)) {
            return e.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("等待事件超时");
}

async fn wait_finished(rt: &Runtime, session: &str, id: &str) -> Value {
    wait_event(rt, session, |e| {
        e.kind == "job.finished" && e.data["id"] == id
    })
    .await
    .data
}

/// 到限转后台后任务继续执行并存续到回合结束，最终以完整输出与退出码结算。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_timeout_hands_off_and_completes_after_turn_ends() {
    let fx = fixture().await;
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            shell_call("c1", "Start-Sleep -Seconds 8; Write-Output finished"),
            ModelAnswer {
                content: "后台已接管".into(),
                ..Default::default()
            },
        ])),
    });
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider)
        .await
        .unwrap();
    // 回合已结束：shell 调用应正常结算，并给出转后台提示而非 TOOL_TIMEOUT。
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "c1")
        .unwrap();
    assert_eq!(result.data["status"], "completed");
    assert_eq!(result.data["isError"], false);
    assert!(
        result.data["content"]
            .as_str()
            .unwrap()
            .contains("background_status"),
        "转后台结果应提示查询工具：{}",
        result.data["content"]
    );
    let started = events
        .iter()
        .find(|e| e.kind == "job.started")
        .expect("应有 job.started 事件");
    let id = started.data["id"].as_str().unwrap().to_owned();
    assert_eq!(
        started.data["command"],
        "Start-Sleep -Seconds 8; Write-Output finished"
    );
    // 回合结束后任务仍存活：最终以 completed + 完整输出结算。
    let finished = wait_finished(&fx.runtime, &fx.session, &id).await;
    assert_eq!(finished["status"], "completed");
    assert!(
        finished["output"]["stdout"]
            .as_str()
            .unwrap()
            .contains("finished")
    );
    assert_eq!(finished["output"]["exit_code"], 0);
    assert!(fx.runtime.jobs.is_empty(), "结算后注册表应清空");
}

/// 转后台任务可经注册表取消：结算为 cancelled，且进程树被收割（无孤儿写文件）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handed_off_job_cancels_without_orphan_process() {
    let fx = fixture().await;
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            shell_call(
                "c1",
                "Start-Sleep -Seconds 30; Set-Content -Path flag.txt -Value done",
            ),
            ModelAnswer {
                content: "后台已接管".into(),
                ..Default::default()
            },
        ])),
    });
    let rt = fx.runtime.clone();
    let session = fx.session.clone();
    let turn = tokio::spawn(async move { rt.run_session(session, "run".into(), provider).await });
    let started = wait_event(&fx.runtime, &fx.session, |e| e.kind == "job.started").await;
    let id = started.data["id"].as_str().unwrap().to_owned();
    let (owner, token) = fx.runtime.jobs.owner_token(&id).unwrap();
    assert_eq!(owner, fx.session, "任务应归属当前会话");
    token.cancel();
    let finished = wait_finished(&fx.runtime, &fx.session, &id).await;
    assert_eq!(finished["status"], "cancelled");
    turn.await.unwrap().unwrap();
    assert!(fx.runtime.jobs.is_empty());
    // 进程树已被收割：取消后再等待片刻，标记文件不应出现。
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !fx.source.join("flag.txt").exists(),
        "取消后不得有孤儿进程写入文件"
    );
}

/// 非 shell 工具到限维持现状：取消并结算为结果未知（TOOL_TIMEOUT 语义回归）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_shell_tool_times_out_unknown() {
    let fx = fixture().await;
    struct Probe;
    #[async_trait]
    impl ExternalTool for Probe {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "probe".into(),
                description: "慢探针".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
            }
        }
        fn requires_approval(&self) -> bool {
            false
        }
        async fn execute(
            &self,
            _: ToolContext,
            _: Value,
            cancel: CancellationToken,
        ) -> AppResult<ToolResult> {
            tokio::select! {
                _ = cancel.cancelled() => Err(harness_runtime::model::cancelled()),
                _ = tokio::time::sleep(Duration::from_secs(30)) => Ok(ToolResult {
            structured_content: None,
                    content: "done".into(), is_error: false, diff: None, escalated: false,
                }),
            }
        }
    }
    let _reg = fx.runtime.register_tool(Arc::new(Probe)).unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            ModelAnswer {
                calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "probe".into(),
                    arguments: json!({}),
                }],
                ..Default::default()
            },
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider)
        .await
        .unwrap();
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "c1")
        .unwrap();
    assert_eq!(result.data["status"], "unknown");
}
