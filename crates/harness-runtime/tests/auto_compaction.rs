//! 请求前自动压缩集成测试：窗口阈值触发、投影重置、usage 记账与失败语义。
use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// 脚本化 provider：声明小窗口便于触发阈值，按序回放回答并记录每次请求的投影。
struct Scripted {
    answers: Mutex<VecDeque<ModelAnswer>>,
    requests: Mutex<Vec<Vec<ModelMessage>>>,
}
impl Scripted {
    fn new(answers: VecDeque<ModelAnswer>) -> Self {
        Self {
            answers: Mutex::new(answers),
            requests: Mutex::new(vec![]),
        }
    }
    fn request_roles(&self, index: usize) -> Vec<String> {
        self.requests.lock().unwrap()[index]
            .iter()
            .map(|m| format!("{:?}", m.role))
            .collect()
    }
}
#[async_trait]
impl ModelProvider for Scripted {
    fn context_window(&self) -> Option<u32> {
        Some(1000)
    }
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.requests.lock().unwrap().push(messages);
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

/// 阈值场景的第一响应：带一个免审批工具调用，推进回合到下一轮循环的压缩检查点。
fn threshold_call(id: &str, prompt: u64) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: "todo_write".into(),
            arguments: json!({"items":[]}),
        }],
        usage: json!({"prompt_tokens":prompt,"completion_tokens":10}),
        ..Default::default()
    }
}

struct Fixture {
    runtime: Arc<Runtime>,
    session: String,
    _dir: tempfile::TempDir,
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
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_runs_before_next_request_after_threshold() {
    let fx = fixture().await;
    let provider = Arc::new(Scripted::new(VecDeque::from([
        // 第一轮：输入 850 ≥ 窗口 1000 × 0.8，带工具调用 → 下一轮请求前应触发压缩。
        threshold_call("c1", 850),
        // 第二轮（压缩摘要请求）：返回纯文本摘要。
        ModelAnswer {
            content: "会话摘要：任务与关键状态".into(),
            usage: json!({"prompt_tokens":50,"completion_tokens":5}),
            ..Default::default()
        },
        // 第三轮：压缩后的正式请求。
        ModelAnswer {
            content: "done".into(),
            ..Default::default()
        },
    ])));
    let session = fx.session.clone();
    let runtime = fx.runtime.clone();
    let scripted = provider.clone();
    let task =
        tokio::spawn(async move { runtime.run_session(session, "run".into(), scripted).await });
    let _ = task.await;

    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    let summaries: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| e.kind == "compaction.summary")
        .collect();
    assert_eq!(summaries.len(), 1, "应恰好触发一次自动压缩");
    assert_eq!(summaries[0].data["trigger"], "auto");
    assert_eq!(summaries[0].data["content"], "会话摘要：任务与关键状态");

    // 第三次请求（正式请求）的投影应基于压缩后的摘要：system + user(摘要)。
    let request_count = provider.requests.lock().unwrap().len();
    assert_eq!(request_count, 3, "模型请求次数：正常 + 压缩 + 正常");
    let final_roles = provider.request_roles(2);
    assert_eq!(
        final_roles.len(),
        2,
        "压缩后投影应重置为摘要（system + user）"
    );

    // 压缩请求本身返回的 usage 应计入回合预算（不得通过压缩绕过预算）。
    let budgets: Vec<&SessionEvent> = events.iter().filter(|e| e.kind == "token.budget").collect();
    let used = budgets
        .last()
        .map(|e| e.data["usedTokens"].as_u64().unwrap_or(0))
        .unwrap_or(0);
    assert!(
        used >= 850 + 10 + 50 + 5,
        "预算应累计主请求与压缩请求的 usage，实际 {used}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_disabled_keeps_existing_behavior() {
    let fx = fixture().await;
    fx.runtime
        .store
        .set("app.auto_compaction", &json!(false))
        .await
        .unwrap();
    let provider = Arc::new(Scripted::new(VecDeque::from([
        threshold_call("c1", 850),
        ModelAnswer {
            content: "done".into(),
            ..Default::default()
        },
    ])));
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider.clone())
        .await
        .unwrap();
    eprintln!("[dbg-test] run_session 已返回");
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    eprintln!("[dbg-test] events 已读取，共 {} 条", events.len());
    assert!(
        !events.iter().any(|e| e.kind == "compaction.summary"),
        "开关关闭时不应自动压缩"
    );
    // 第二次请求仍是完整投影（system + user + assistant + tool），未被压缩。
    let request_count = provider.requests.lock().unwrap().len();
    assert_eq!(request_count, 2);
    assert!(provider.request_roles(1).len() > 2, "投影未被压缩重置");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_failure_keeps_projection_and_continues() {
    let fx = fixture().await;
    // 摘要请求返回空内容（COMPACTION_INVALID 语义）。
    let provider = Arc::new(Scripted::new(VecDeque::from([
        threshold_call("c1", 850),
        ModelAnswer {
            content: "".to_string(), // 无效摘要
            usage: json!({"prompt_tokens":50,"completion_tokens":0}),
            ..Default::default()
        },
        ModelAnswer {
            content: "done".into(),
            ..Default::default()
        },
    ])));
    fx.runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider.clone())
        .await
        .unwrap();
    let events = fx.runtime.store.events(&fx.session, 0).await.unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "compaction.summary"),
        "摘要无效时不落压缩事件、不替换有效投影"
    );
    let attempts: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| e.kind == "assistant.attempt")
        .collect();
    assert!(
        attempts
            .iter()
            .any(|e| !e.data["error"].is_null() && e.data["error"].to_string().contains("压缩")),
        "压缩失败应可观察（assistant.attempt 记录错误）"
    );
    // 剩余余量下继续原请求：第三次请求不是摘要投影。
    let request_count = provider.requests.lock().unwrap().len();
    assert_eq!(request_count, 3, "压缩失败但仍有余量时继续原请求");
    assert!(provider.request_roles(2).len() > 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_failure_at_window_limit_returns_diagnosable_error() {
    let fx = fixture().await;
    // 输入 1200 已超窗口 1000：压缩失败后继续请求必然溢出，应返回可诊断错误。
    let provider = Arc::new(Scripted::new(VecDeque::from([
        threshold_call("c1", 1200),
        ModelAnswer {
            content: "".to_string(), // 压缩失败
            ..Default::default()
        },
    ])));
    let result = fx
        .runtime
        .clone()
        .run_session(fx.session.clone(), "run".into(), provider)
        .await;
    let error = result.unwrap_err();
    assert_eq!(error.code, "CONTEXT_WINDOW_EXCEEDED");
}
