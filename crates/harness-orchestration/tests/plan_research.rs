use async_trait::async_trait;
use harness_contracts::{AppResult, ApprovalPolicy, ExecutionMode, ToolCall, ToolDefinition};
use harness_orchestration::install;
use harness_runtime::{
    Runtime,
    model::{DeltaSink, ModelAnswer, ModelMessage, ModelProvider},
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// 父会话首条用户任务标记：父会话每一轮投影都携带，子代理消息中不出现。
const PARENT_MARKER: &str = "调研并汇报";
/// 子代理派生任务标记：只出现在子代理的首条用户消息里。
const CHILD_MARKER: &str = "总结样例文档";
/// 子代理最终报告正文：断言经真实 agent_wait 回流到父会话。
const REPORT: &str = "调研结论样例正文";

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

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

/// 从父会话投影文本中提取 spawn 结果里的子会话 id（工具结果 JSON 为必然载体）。
fn first_session_id(text: &str) -> Option<String> {
    let start = text.find("\"sessionId\":\"")? + "\"sessionId\":\"".len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// 父会话第一步派生调研子代理，第二步用真实 sessionId 调 agent_wait 收集报告；
/// 子代理按任务标记路由，最终以 assistant 消息回报正文。
struct Router {
    parent_step: Mutex<u32>,
    child: Mutex<VecDeque<ModelAnswer>>,
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
            let mut step = self.parent_step.lock().unwrap();
            *step += 1;
            match *step {
                1 => Ok(answer(vec![call(
                    "spawn",
                    "agent_spawn",
                    json!({"task": format!("{CHILD_MARKER} 并回报"), "title": "调研"}),
                )])),
                2 => {
                    let child = first_session_id(&text)
                        .unwrap_or_else(|| panic!("父会话投影中应能取到子会话 id"));
                    Ok(answer(vec![call(
                        "wait",
                        "agent_wait",
                        json!({"sessionId": child, "timeoutMs": 30000}),
                    )]))
                }
                _ => Ok(empty_answer()),
            }
        } else if text.contains(CHILD_MARKER) {
            Ok(self
                .child
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(empty_answer))
        } else {
            Ok(empty_answer())
        }
    }
}

#[tokio::test]
async fn plan_mode_agent_wait_collects_research_report() {
    let temp = tempfile::tempdir().unwrap();
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
    // 编排工具宿主保持存活（与桌面/CLI 装载方式一致）。
    let _installed = install(runtime.clone()).unwrap();
    runtime.session_plan_set(&session.id, true).await.unwrap();
    let router = Arc::new(Router {
        parent_step: Mutex::new(0),
        child: Mutex::new(VecDeque::from(vec![ModelAnswer {
            content: REPORT.into(),
            usage: json!({"prompt_tokens":0,"completion_tokens":0}),
            ..Default::default()
        }])),
    });
    runtime
        .clone()
        .run_session(session.id.clone(), PARENT_MARKER.into(), router)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let created = events
        .iter()
        .find(|e| e.kind == "subagent.created")
        .expect("应记录 subagent.created");
    assert_eq!(created.data["planMode"], json!(true));
    let child = created.data["sessionId"].as_str().unwrap();
    assert!(
        runtime.session_plan_get(child).await.unwrap(),
        "调研子代理应被强制进入计划模式"
    );
    for id in ["spawn", "wait"] {
        assert!(
            events.iter().any(|event| event.kind == "tool.result"
                && event.data["callId"] == id
                && event.data["isError"] == false),
            "计划模式应放行 {id} 且执行成功"
        );
    }
    assert!(
        events.iter().all(|event| event.kind != "approval.request"),
        "计划模式调研闭环不应产生审批"
    );
    let wait_result = events
        .iter()
        .find(|event| event.kind == "tool.result" && event.data["callId"] == "wait")
        .unwrap();
    assert!(
        wait_result.data["content"]
            .as_str()
            .unwrap()
            .contains(REPORT),
        "agent_wait 应回流子代理报告正文"
    );
}
