use async_trait::async_trait;
use harness_contracts::{
    AppResult, ApprovalPolicy, AskAnswer, AskSource, ExecutionMode, ToolCall, ToolDefinition,
    ToolResult,
};
use harness_runtime::{
    ExternalTool, Runtime, ToolContext,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 父会话首条用户任务标记：父会话每一轮投影都携带，子代理消息中不出现。
const PARENT_MARKER: &str = "调研并汇报";
/// 子代理派生任务标记：只出现在子代理的首条用户消息里。
const CHILD_MARKER: &str = "调查只读目录";

fn empty_answer() -> ModelAnswer {
    ModelAnswer {
        usage: json!({"prompt_tokens":0,"completion_tokens":0}),
        ..Default::default()
    }
}

fn answer(calls: Vec<ToolCall>) -> ModelAnswer {
    ModelAnswer {
        calls,
        usage: json!({"prompt_tokens":0,"completion_tokens":0}),
        ..Default::default()
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

/// 按消息内容路由父子脚本：父会话投影永远包含首条用户任务（父标记），
/// 子代理投影包含派生任务文本（子标记），两者互不重叠，避免父子竞争同一脚本。
struct Router {
    parent: Mutex<VecDeque<ModelAnswer>>,
    child: Mutex<VecDeque<ModelAnswer>>,
}

impl Router {
    fn new(parent: Vec<ModelAnswer>, child: Vec<ModelAnswer>) -> Self {
        Self {
            parent: Mutex::new(parent.into()),
            child: Mutex::new(child.into()),
        }
    }

    fn pop(queue: &Mutex<VecDeque<ModelAnswer>>) -> ModelAnswer {
        queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(empty_answer)
    }
}

#[async_trait]
impl ModelProvider for Router {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.contains(PARENT_MARKER) {
            Ok(Self::pop(&self.parent))
        } else if text.contains(CHILD_MARKER) {
            Ok(Self::pop(&self.child))
        } else {
            Ok(empty_answer())
        }
    }
}

/// 忽略消息内容、按队列出队并记录每轮暴露的工具名，供暴露断言使用。
struct NamesProbe(Mutex<VecDeque<ModelAnswer>>, Arc<Mutex<Vec<Vec<String>>>>);

#[async_trait]
impl ModelProvider for NamesProbe {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        tools: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.1
            .lock()
            .unwrap()
            .push(tools.into_iter().map(|t| t.name).collect());
        Ok(self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(empty_answer))
    }
}

/// 忽略消息内容、按队列出队的最简脚本 provider。
struct Scripted(Mutex<VecDeque<ModelAnswer>>);

#[async_trait]
impl ModelProvider for Scripted {
    async fn complete(
        &self,
        _: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(empty_answer))
    }
}

/// 计划模式信任边界探针：验证白名单按名放行外部工具（agent_wait 与 orchestration
/// 注册机制共用一条路径），agent_send 之类未受信扩展仍被拒绝。
struct StubTool {
    name: &'static str,
}

#[async_trait]
impl ExternalTool for StubTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "fixture".into(),
            parameters: json!({"type":"object"}),
        }
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> AppResult<ToolResult> {
        Ok(ToolResult {
            content: "stub ok".into(),
            is_error: false,
            diff: None,
            escalated: false,
        })
    }
}

async fn fixture(temp: &tempfile::TempDir) -> (Arc<Runtime>, String, std::path::PathBuf) {
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("data")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "plan")
        .await
        .unwrap();
    runtime.set_approval_policy(&session.id, Some(ApprovalPolicy::Auto));
    (
        runtime,
        session.id,
        std::path::PathBuf::from(&workspace.replica_path),
    )
}

/// 等待子代理回合结束（最后一个 turn.end 晚于最后一个 turn.start）。
async fn wait_child_settled(runtime: &Runtime, child: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let events = runtime.store.events(child, 0).await.unwrap();
        let start = events.iter().rposition(|e| e.kind == "turn.start");
        let end = events.iter().rposition(|e| e.kind == "turn.end");
        if end.is_some_and(|e| start.is_none_or(|s| e > s)) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "子代理未在期限内结束"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn plan_mode_spawn_forces_child_into_readonly_plan() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id, replica) = fixture(&temp).await;
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let router = Arc::new(Router::new(
        vec![answer(vec![call(
            "spawn",
            "agent_spawn",
            json!({"task": format!("{CHILD_MARKER} 并汇报"), "title": "调研"}),
        )])],
        vec![
            answer(vec![call(
                "w1",
                "write",
                json!({"path":"evil.txt","content":"side effect"}),
            )]),
            empty_answer(),
        ],
    ));
    runtime
        .clone()
        .run_session(session_id.clone(), PARENT_MARKER.into(), router)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    let created = events
        .iter()
        .find(|e| e.kind == "subagent.created")
        .expect("应记录 subagent.created");
    assert_eq!(
        created.data["planMode"],
        json!(true),
        "计划模式派生应带 planMode 标记"
    );
    assert!(
        events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == "spawn"
            && event.data["isError"] == false),
        "计划模式应放行 agent_spawn"
    );
    let child = created.data["sessionId"].as_str().unwrap().to_owned();
    assert!(
        runtime.session_plan_get(&child).await.unwrap(),
        "调研子代理应被强制进入计划模式"
    );
    wait_child_settled(&runtime, &child).await;
    let child_events = runtime.store.events(&child, 0).await.unwrap();
    assert!(
        child_events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == "w1"
            && event.data["status"] == "denied"),
        "子代理写操作应被计划模式拒绝"
    );
    assert!(
        !replica.join("evil.txt").exists(),
        "子代理不得产生文件副作用"
    );
}

#[tokio::test]
async fn plan_mode_exposes_trusted_agent_wait_without_approval() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id, _) = fixture(&temp).await;
    let _wait = runtime
        .register_tool(Arc::new(StubTool { name: "agent_wait" }))
        .unwrap();
    let _send = runtime
        .register_tool(Arc::new(StubTool { name: "agent_send" }))
        .unwrap();
    let names = Arc::new(Mutex::new(Vec::new()));
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let probe = Arc::new(NamesProbe(
        Mutex::new(VecDeque::from(vec![answer(vec![call(
            "w",
            "agent_wait",
            json!({"sessionId":"x"}),
        )])])),
        names.clone(),
    ));
    runtime
        .clone()
        .run_session(session_id.clone(), "调查".into(), probe)
        .await
        .unwrap();
    let exposed = names.lock().unwrap()[0].clone();
    assert!(
        exposed.iter().any(|n| n == "agent_wait"),
        "计划模式应暴露受信 agent_wait"
    );
    assert!(
        exposed.iter().all(|n| n != "agent_send"),
        "计划模式不得暴露未受信扩展"
    );
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    assert!(
        events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == "w"
            && event.data["isError"] == false),
        "受信 agent_wait 在计划模式应放行执行"
    );
    assert!(
        events.iter().all(|event| event.kind != "approval.request"),
        "计划模式白名单工具不应产生审批"
    );
}

#[tokio::test]
async fn plan_mode_denies_untrusted_agent_send() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id, _) = fixture(&temp).await;
    let _send = runtime
        .register_tool(Arc::new(StubTool { name: "agent_send" }))
        .unwrap();
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let provider = Arc::new(Scripted(Mutex::new(VecDeque::from(vec![answer(vec![
        call("send", "agent_send", json!({"sessionId":"x","task":"y"})),
    ])]))));
    runtime
        .clone()
        .run_session(session_id.clone(), "调查".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    // 计划模式只暴露受信名单，未受信扩展不在定义查找范围内，必然以错误结果收场。
    assert!(
        events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == "send"
            && event.data["isError"] == true),
        "未受信扩展在计划模式不可执行"
    );
}

#[tokio::test]
async fn plan_mode_reentry_refuses_running_research_child_then_allows_finished() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id, _) = fixture(&temp).await;
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let router = Arc::new(Router::new(
        vec![answer(vec![call(
            "spawn",
            "agent_spawn",
            json!({"task": format!("{CHILD_MARKER} 并汇报"), "title": "调研"}),
        )])],
        vec![
            answer(vec![call(
                "wait",
                "ask_user",
                json!({"question":"等待指示"}),
            )]),
            empty_answer(),
        ],
    ));
    runtime
        .clone()
        .run_session(session_id.clone(), PARENT_MARKER.into(), router)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    let child = events
        .iter()
        .find(|e| e.kind == "subagent.created")
        .unwrap()
        .data["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    // 子代理挂在 ask_user 上：退出计划模式不受影响，但重新进入必须被拒。
    let request = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(request) = runtime.questions.list().into_iter().next() {
                break request;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    runtime.session_plan_set(&session_id, false).await.unwrap();
    assert_eq!(
        runtime
            .session_plan_set(&session_id, true)
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_UNVERIFIED",
        "运行中的调研子代理应阻塞重新进入"
    );
    runtime
        .questions
        .resolve(
            &request.id,
            AskAnswer {
                text: "继续".into(),
                source: AskSource::Selected,
            },
        )
        .unwrap();
    wait_child_settled(&runtime, &child).await;
    // turn.end 落库后 active 表清理略有滞后，留少量重试避免时序假失败。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match runtime.session_plan_set(&session_id, true).await {
            Ok(()) => break,
            Err(e) if e.code == "PLAN_MODE_UNVERIFIED" => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "已完成的调研子代理不应阻塞重新进入"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("意外错误：{e}"),
        }
    }
    assert!(runtime.session_plan_get(&session_id).await.unwrap());
}

#[tokio::test]
async fn non_plan_spawn_still_requires_approval_and_send_stays_exposed() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id, _) = fixture(&temp).await;
    let _send = runtime
        .register_tool(Arc::new(StubTool { name: "agent_send" }))
        .unwrap();
    let names = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::new(NamesProbe(
        Mutex::new(VecDeque::from(vec![answer(vec![
            call("s", "agent_spawn", json!({"task":"x"})),
            call("send", "agent_send", json!({"sessionId":"nope","task":"y"})),
        ])])),
        names.clone(),
    ));
    runtime
        .clone()
        .run_session(session_id.clone(), "常规任务".into(), probe)
        .await
        .unwrap();
    assert!(
        names.lock().unwrap()[0].iter().any(|n| n == "agent_send"),
        "非计划模式扩展暴露行为不变"
    );
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "approval.request" && event.data["tool"] == "agent_spawn"),
        "非计划模式 agent_spawn 仍应走审批"
    );
    assert!(
        events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == "send"
            && event.data["isError"] == false),
        "非计划模式扩展行为不变"
    );
}
