//! 工具链路测试：假后端验证 browser_* 工具注册、审批与执行，不启动真实浏览器。
use async_trait::async_trait;
use harness_browser::backend::BrowserBackend;
use harness_browser::command::{BrowserCommand, CommandOutcome};
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, MessageRole, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

type Recorder = Arc<Mutex<Vec<BrowserCommand>>>;

struct RecordedBackend {
    commands: Recorder,
}

#[async_trait]
impl BrowserBackend for RecordedBackend {
    async fn execute(&self, command: BrowserCommand) -> AppResult<CommandOutcome> {
        self.commands.lock().unwrap().push(command);
        Ok(CommandOutcome {
            summary: "fake ok".into(),
            ..Default::default()
        })
    }
    async fn close(&self) -> AppResult<()> {
        Ok(())
    }
}

struct RecordedFactory {
    commands: Recorder,
}

impl harness_browser::BackendFactory for RecordedFactory {
    fn create(&self) -> futures::future::BoxFuture<'_, AppResult<Arc<dyn BrowserBackend>>> {
        let commands = self.commands.clone();
        Box::pin(async move { Ok(Arc::new(RecordedBackend { commands }) as _) })
    }
}

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
    requests: Mutex<Vec<Vec<ModelMessage>>>,
    tool_names: Mutex<Vec<String>>,
}

#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        tools: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.requests.lock().unwrap().push(messages);
        self.tool_names
            .lock()
            .unwrap()
            .extend(tools.into_iter().map(|t| t.name));
        let mut answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))?;
        if answer.usage.is_null() {
            answer.usage = serde_json::json!({"prompt_tokens":0,"completion_tokens":0});
        }
        Ok(answer)
    }
}

fn call(id: &str, name: &str, args: Value) -> ModelAnswer {
    ModelAnswer {
        calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }],
        ..Default::default()
    }
}

async fn fixture() -> (Arc<Runtime>, harness_contracts::SessionInfo) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "browser")
        .await
        .unwrap();
    (runtime, session)
}

#[tokio::test]
async fn browser_tools_are_registered_and_go_through_approval() {
    let (runtime, session) = fixture().await;
    let commands: Recorder = Arc::default();
    let _owner = harness_browser::install_with(
        runtime.clone(),
        RecordedFactory {
            commands: commands.clone(),
        },
    )
    .unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "browser_navigate",
                json!({"url": "http://localhost:9/"}),
            ),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
        tool_names: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "打开页面".into(),
        model.clone(),
    ));
    let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(r) = runtime.approvals.list().first() {
                break r.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending.tool, "browser_navigate");
    runtime.approvals.resolve(&pending.id, true).unwrap();
    task.await.unwrap().unwrap();

    assert_eq!(commands.lock().unwrap().len(), 1);
    // 八个浏览器工具全部出现在模型可见的工具定义里
    let names = model.tool_names.lock().unwrap();
    for name in [
        "browser_navigate",
        "browser_snapshot",
        "browser_click",
        "browser_type",
        "browser_press",
        "browser_screenshot",
        "browser_evaluate",
        "browser_close",
    ] {
        assert!(names.iter().any(|n| n == name), "缺少工具 {name}");
    }
    // 工具结果回传给模型
    assert!(
        model.requests.lock().unwrap()[1]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content.contains("fake ok"))
    );
}

#[tokio::test]
async fn denied_browser_call_never_reaches_backend() {
    let (runtime, session) = fixture().await;
    let commands: Recorder = Arc::default();
    let _owner = harness_browser::install_with(
        runtime.clone(),
        RecordedFactory {
            commands: commands.clone(),
        },
    )
    .unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "browser_evaluate", json!({"expression": "1+1"})),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
        tool_names: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "试着执行".into(),
        model,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(r) = runtime.approvals.list().first() {
                runtime.approvals.resolve(&r.id, false).unwrap();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.await.unwrap().unwrap();
    assert!(commands.lock().unwrap().is_empty());
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().any(|e| {
        e.kind == "tool.result"
            && e.data["isError"] == true
            && e.data["content"]
                .as_str()
                .unwrap()
                .contains("APPROVAL_DENIED")
    }));
}
