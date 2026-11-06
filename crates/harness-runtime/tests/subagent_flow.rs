use async_trait::async_trait;
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
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
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AppError::new("FIXTURE", "unexpected extra request"))
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
async fn setup(name: &str) -> (tempfile::TempDir, Arc<Runtime>, String) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, name)
        .await
        .unwrap();
    (temp, runtime, session.id)
}
/// agent_spawn 需要审批；测试内自动批准，避免等待审批超时。
/// 持 Weak 引用，不阻止 Runtime 释放 host.lock。
fn auto_approve(runtime: &Arc<Runtime>) {
    let runtime = Arc::downgrade(runtime);
    tokio::spawn(async move {
        loop {
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            if let Some(request) = runtime.approvals.list().first() {
                runtime.approvals.resolve(&request.id, true).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

#[tokio::test]
async fn agent_spawn_with_unknown_model_is_rejected_without_creating_child() {
    let (_temp, runtime, session) = setup("spawn-invalid").await;
    let workspace_id = runtime.store.session(&session).await.unwrap().workspace_id;
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "agent_spawn",
                json!({"task":"子任务","modelId":"missing"}),
            ),
            ModelAnswer::default(),
        ])),
    });
    auto_approve(&runtime);
    runtime
        .clone()
        .run_session(session.clone(), "parent".into(), model)
        .await
        .unwrap();
    let events = runtime.store.events(&session, 0).await.unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "c1")
        .unwrap();
    assert_eq!(result.data["isError"], true);
    assert!(
        result.data["content"]
            .as_str()
            .unwrap()
            .contains("MODEL_NOT_FOUND")
    );
    let sessions = runtime.store.sessions(Some(&workspace_id)).await.unwrap();
    assert_eq!(sessions.len(), 1, "无效 modelId 不得创建子会话");
    assert!(runtime.children_of(&session).is_empty());
}

#[tokio::test]
async fn agent_spawn_with_model_configures_child_provider_and_records_link() {
    let (_temp, runtime, session) = setup("spawn-model").await;
    let mut supplier = runtime.models.copy_provider("deepseek").await.unwrap();
    supplier.endpoints = vec![ProtocolEndpoint {
        api_mode: ApiMode::ChatCompletions,
        base_url: "http://127.0.0.1:1".into(),
    }];
    let supplier = runtime.models.save_provider(supplier, None).await.unwrap();
    let model = runtime
        .models
        .save_model(
            ModelConfig {
                compatibility: ProviderCompatibility::Standard,
                id: String::new(),
                name: "child".into(),
                provider_id: supplier.id,
                api_mode: ApiMode::ChatCompletions,
                max_output_tokens: None,
                base_url: String::new(),
                model: "m".into(),
                credential_ref: String::new(),
                proxy: None,
            },
            None,
        )
        .await
        .unwrap();
    let parent = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "agent_spawn",
                json!({"task":"子任务","modelId":model.id}),
            ),
            ModelAnswer::default(),
        ])),
    });
    auto_approve(&runtime);
    runtime
        .clone()
        .run_session(session.clone(), "parent".into(), parent)
        .await
        .unwrap();
    let child = runtime.children_of(&session).pop().expect("子会话应已登记");
    let events = runtime.store.events(&session, 0).await.unwrap();
    let created = events
        .iter()
        .find(|e| e.kind == "subagent.created")
        .unwrap();
    assert_eq!(created.data["sessionId"], child.as_str());
    assert_eq!(created.data["modelId"], model.id.as_str());
    let result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "c1")
        .unwrap();
    assert_eq!(result.data["isError"], false);
}

#[tokio::test]
async fn parent_link_survives_restart_and_decorates_session_list() {
    let (temp, runtime, session) = setup("restart").await;
    let parent = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "agent_spawn", json!({"task":"子任务"})),
            ModelAnswer::default(),
            // 不传 modelId 时子代理复用父 provider：父会话第 2 轮与子代理第 1 轮
            // 共产生 3 次 complete，若队列只有 2 个回答，两者会竞争最后一个，
            // CI 慢机时序下父会话可能从空队列 pop 而报 FIXTURE 错。
            ModelAnswer::default(),
        ])),
    });
    auto_approve(&runtime);
    runtime
        .clone()
        .run_session(session.clone(), "parent".into(), parent)
        .await
        .unwrap();
    let child = runtime.children_of(&session).pop().expect("子会话应已登记");
    assert_eq!(runtime.parent_of(&child).as_deref(), Some(session.as_str()));
    drop(runtime);
    // 子代理后台任务短暂持有 Arc<Runtime>，host.lock 释放存在窗口，须重试。
    let mut reopened = None;
    for _ in 0..100 {
        match Runtime::open(temp.path().join("host")).await {
            Ok(value) => {
                reopened = Some(value);
                break;
            }
            Err(e) if e.code == "HOST_RUNNING" => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => panic!("{e}"),
        }
    }
    let reopened = reopened.expect("重启后应能重新打开数据目录");
    assert_eq!(
        reopened.parent_of(&child).as_deref(),
        Some(session.as_str()),
        "重启后父子关系应从事件重建"
    );
    assert!(reopened.children_of(&session).contains(&child));
    let listed = reopened.list_sessions(None).await.unwrap();
    let child_row = listed.iter().find(|s| s.id == child).unwrap();
    assert_eq!(child_row.parent_id.as_deref(), Some(session.as_str()));
    let parent_row = listed.iter().find(|s| s.id == session).unwrap();
    assert!(parent_row.parent_id.is_none());
}
