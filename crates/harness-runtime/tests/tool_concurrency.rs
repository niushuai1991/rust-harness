//! 工具并发调度集成测试：白名单并发段的执行重叠、上限、屏障、取消结算与开关回退。
use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    ExternalTool, Registration, Runtime, ToolContext,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider, cancelled},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
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

fn batch(calls: Vec<(&str, &str)>) -> ModelAnswer {
    ModelAnswer {
        calls: calls
            .into_iter()
            .map(|(id, name)| ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: json!({}),
            })
            .collect(),
        ..Default::default()
    }
}

/// 工具执行时间线：记录进入/退出时刻，用于推断重叠与并发峰值。
#[derive(Default)]
struct Timeline {
    entries: Mutex<Vec<(&'static str, &'static str, Instant)>>,
}
impl Timeline {
    fn record(&self, name: &'static str, phase: &'static str) {
        self.entries
            .lock()
            .unwrap()
            .push((name, phase, Instant::now()));
    }
    fn tool_entries(&self, name: &str) -> Vec<(&'static str, &'static str, Instant)> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _, _)| *n == name)
            .copied()
            .collect()
    }
    /// 两个工具的执行区间是否发生过重叠。
    fn overlapped(&self, a: &str, b: &str) -> bool {
        let span = |name: &str| {
            let entries = self.tool_entries(name);
            let start = entries.iter().find(|(_, p, _)| *p == "enter").map(|e| e.2);
            let end = entries.iter().find(|(_, p, _)| *p == "exit").map(|e| e.2);
            (start, end)
        };
        let (a_start, a_end) = span(a);
        let (b_start, b_end) = span(b);
        match (a_start, a_end, b_start, b_end) {
            (Some(as_), Some(ae), Some(bs), Some(be)) => as_ < be && bs < ae,
            _ => false,
        }
    }
    /// 同一时刻在场的工具数峰值。
    fn peak_in_flight(&self) -> usize {
        let mut events: Vec<(Instant, i64)> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|(_, phase, at)| (*at, if *phase == "enter" { 1 } else { -1 }))
            .collect();
        events.sort_by_key(|(at, delta)| (*at, -*delta));
        let (mut current, mut peak) = (0usize, 0usize);
        for (_, delta) in events {
            if delta > 0 {
                current += 1;
                peak = peak.max(current);
            } else {
                current = current.saturating_sub(1);
            }
        }
        peak
    }
}

/// 并发探针工具：免审批、声明并发安全，执行时记录时间线并响应取消。
struct ProbeTool {
    name: &'static str,
    delay: Duration,
    timeline: Arc<Timeline>,
}
impl ProbeTool {
    fn register(
        runtime: &Arc<Runtime>,
        name: &'static str,
        delay: Duration,
        timeline: &Arc<Timeline>,
    ) -> Registration {
        runtime
            .register_tool(Arc::new(ProbeTool {
                name,
                delay,
                timeline: timeline.clone(),
            }))
            .unwrap()
    }
}
#[async_trait]
impl ExternalTool for ProbeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "并发探针：记录进入/退出时间线".into(),
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
    fn requires_approval(&self) -> bool {
        false
    }
    fn concurrency_safe(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _: ToolContext,
        _: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        self.timeline.record(self.name, "enter");
        tokio::select! {
            // 被取消的工具返回 CANCELLED：执行守卫的两个分支都结算 unknown，消除 select 竞态歧义。
            _ = cancel.cancelled() => {
                self.timeline.record(self.name, "exit");
                return Err(cancelled());
            }
            _ = tokio::time::sleep(self.delay) => {}
        }
        self.timeline.record(self.name, "exit");
        Ok(ToolResult {
            content: format!("{} done", self.name),
            is_error: false,
            diff: None,
            escalated: false,
        })
    }
}

struct Fixture {
    runtime: Arc<Runtime>,
    session: String,
    _dir: tempfile::TempDir,
    #[allow(dead_code)] // 部分场景不写入源目录
    source: PathBuf,
}
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    Fixture {
        runtime,
        session: session.id,
        _dir: dir,
        source,
    }
}

fn tool_results(events: &[SessionEvent]) -> Vec<(String, String, String)> {
    // (callId, status, seq 顺序即声明顺序的观察点)
    events
        .iter()
        .filter(|e| e.kind == "tool.result")
        .map(|e| {
            (
                e.data["callId"].as_str().unwrap_or_default().to_string(),
                e.data["status"].as_str().unwrap_or_default().to_string(),
                e.seq.clone(),
            )
        })
        .collect()
}

async fn wait_timeline(timeline: &Timeline, name: &str, phase: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if timeline
                .tool_entries(name)
                .iter()
                .any(|(_, p, _)| *p == phase)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_probe_tools_overlap_and_settle_in_declared_order() {
    let fx = fixture().await;
    let timeline = Arc::new(Timeline::default());
    let _a = ProbeTool::register(
        &fx.runtime,
        "probe_a",
        Duration::from_millis(150),
        &timeline,
    );
    let _b = ProbeTool::register(
        &fx.runtime,
        "probe_b",
        Duration::from_millis(150),
        &timeline,
    );
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            batch(vec![("c1", "probe_a"), ("c2", "probe_b")]),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let started = Instant::now();
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider)
        .await
        .unwrap();
    let elapsed = started.elapsed();
    // 串行总时长 ≥300ms；并发生效则两个 150ms 探针重叠，明显低于串行和。
    assert!(
        elapsed < Duration::from_millis(280),
        "探针应并发执行，实际耗时 {elapsed:?}"
    );
    assert!(
        timeline.overlapped("probe_a", "probe_b"),
        "两个探针的执行区间应重叠"
    );
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let results = tool_results(&events);
    assert_eq!(results.len(), 2);
    // 结果按声明顺序结算：c1 的结果事件先于 c2。
    assert_eq!(results[0].0, "c1");
    assert_eq!(results[1].0, "c2");
    assert!(results.iter().all(|(_, status, _)| status == "completed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_limit_caps_peak_in_flight() {
    let fx = fixture().await;
    fx.runtime
        .store
        .set("app.tool_concurrency_limit", &json!(2))
        .await
        .unwrap();
    let timeline = Arc::new(Timeline::default());
    let mut registrations = Vec::new();
    for (name, id) in [
        ("probe_a", "c1"),
        ("probe_b", "c2"),
        ("probe_c", "c3"),
        ("probe_d", "c4"),
    ] {
        registrations.push(ProbeTool::register(
            &fx.runtime,
            name,
            Duration::from_millis(100),
            &timeline,
        ));
        let _ = id;
    }
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            batch(vec![
                ("c1", "probe_a"),
                ("c2", "probe_b"),
                ("c3", "probe_c"),
                ("c4", "probe_d"),
            ]),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let started = Instant::now();
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider)
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(
        timeline.peak_in_flight() <= 2,
        "同时在场工具数不得超过上限 2"
    );
    assert!(
        elapsed < Duration::from_millis(350),
        "4 个 100ms 探针在上限 2 下应两波完成（约 200ms），实际 {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_barrier_keeps_probes_serial() {
    let fx = fixture().await;
    fx.runtime
        .set_default_approval_policy(Some(ApprovalPolicy::Auto));
    let timeline = Arc::new(Timeline::default());
    let _a = ProbeTool::register(
        &fx.runtime,
        "probe_a",
        Duration::from_millis(100),
        &timeline,
    );
    let _b = ProbeTool::register(
        &fx.runtime,
        "probe_b",
        Duration::from_millis(100),
        &timeline,
    );
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            ModelAnswer {
                calls: vec![
                    ToolCall {
                        id: "c1".into(),
                        name: "probe_a".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "c2".into(),
                        name: "write".into(),
                        arguments: json!({"path":"w.txt","content":"x"}),
                    },
                    ToolCall {
                        id: "c3".into(),
                        name: "probe_b".into(),
                        arguments: json!({}),
                    },
                ],
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
    assert!(
        !timeline.overlapped("probe_a", "probe_b"),
        "屏障（write）两侧的探针不得并发"
    );
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let results = tool_results(&events);
    assert_eq!(results.len(), 3);
    assert_eq!(
        results
            .iter()
            .find(|(id, _, _)| id == "c2")
            .map(|(_, status, _)| status.as_str()),
        Some("completed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_concurrent_batch_settles_unknown_in_order() {
    let fx = fixture().await;
    let timeline = Arc::new(Timeline::default());
    let _a = ProbeTool::register(&fx.runtime, "probe_a", Duration::from_secs(30), &timeline);
    let _b = ProbeTool::register(&fx.runtime, "probe_b", Duration::from_secs(30), &timeline);
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            batch(vec![("c1", "probe_a"), ("c2", "probe_b")]),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let session = fx.session.clone();
    let runtime = fx.runtime.clone();
    let task =
        tokio::spawn(async move { runtime.run_session(session, "run".into(), provider).await });
    wait_timeline(&timeline, "probe_a", "enter").await;
    wait_timeline(&timeline, "probe_b", "enter").await;
    fx.runtime.cancel(&fx.session);
    let _ = task.await;
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let results = tool_results(&events);
    assert_eq!(results.len(), 2, "取消后每个已接收调用都应有唯一结果");
    assert_eq!(results[0].0, "c1");
    assert_eq!(results[1].0, "c2");
    assert!(
        results.iter().all(|(_, status, _)| status == "unknown"),
        "被取消的调用应结算为结果未知，实际 {results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_switch_runs_serial() {
    let fx = fixture().await;
    fx.runtime
        .store
        .set("app.tool_concurrency", &json!(false))
        .await
        .unwrap();
    let timeline = Arc::new(Timeline::default());
    let _a = ProbeTool::register(
        &fx.runtime,
        "probe_a",
        Duration::from_millis(100),
        &timeline,
    );
    let _b = ProbeTool::register(
        &fx.runtime,
        "probe_b",
        Duration::from_millis(100),
        &timeline,
    );
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            batch(vec![("c1", "probe_a"), ("c2", "probe_b")]),
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
    assert!(
        !timeline.overlapped("probe_a", "probe_b"),
        "开关关闭时应回退串行执行"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reads_then_edit_passes_read_before_edit() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    // 副本在创建工作区时生成：文件必须先于工作区存在。
    std::fs::write(source.join("a.txt"), "hello").unwrap();
    std::fs::write(source.join("b.txt"), "world").unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    runtime.set_default_approval_policy(Some(ApprovalPolicy::Auto));
    let replica = workspace.replica_path.clone();
    let fx = Fixture {
        runtime,
        session: session.id,
        _dir: dir,
        source,
    };
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            ModelAnswer {
                calls: vec![
                    ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: json!({"path":"a.txt"}),
                    },
                    ToolCall {
                        id: "c2".into(),
                        name: "read".into(),
                        arguments: json!({"path":"b.txt"}),
                    },
                ],
                ..Default::default()
            },
            ModelAnswer {
                calls: vec![ToolCall {
                    id: "c3".into(),
                    name: "edit".into(),
                    arguments: json!({"path":"a.txt","oldText":"hello","newText":"hi"}),
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
    let results = tool_results(&events);
    assert_eq!(results.len(), 3);
    let edit = results.iter().find(|(id, _, _)| id == "c3").unwrap();
    assert_eq!(
        edit.1, "completed",
        "并发 read 记录后 edit 应通过 read-before-edit"
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&replica).join("a.txt")).unwrap(),
        "hi"
    );
}
