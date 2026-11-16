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
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn plan_state_persists_across_restart_and_rejects_unknown_session() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let data = temp.path().join("data");
    let runtime = Runtime::open(data.clone()).await.unwrap();
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
    assert!(!runtime.session_plan_get(&session.id).await.unwrap());
    runtime.session_plan_set(&session.id, true).await.unwrap();
    assert!(runtime.session_plan_get(&session.id).await.unwrap());
    assert!(runtime.session_plan_set("missing", true).await.is_err());
    drop(runtime);
    let runtime = Runtime::open(data).await.unwrap();
    assert!(runtime.session_plan_get(&session.id).await.unwrap());
    runtime.session_plan_set(&session.id, false).await.unwrap();
    assert!(!runtime.session_plan_get(&session.id).await.unwrap());
}

#[tokio::test]
async fn plan_refuses_to_initialize_worktree_even_after_restoring_mode() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join(".git")).unwrap();
    let runtime = Runtime::open(temp.path().join("data")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Worktree)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "plan")
        .await
        .unwrap();
    assert_eq!(
        runtime
            .session_plan_set(&session.id, true)
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_WORKTREE"
    );
    runtime
        .append(&session.id, "session.plan", json!({"enabled":true}))
        .await
        .unwrap();
    assert_eq!(
        runtime
            .clone()
            .run_session(
                session.id,
                "plan".into(),
                Arc::new(Provider(Mutex::new(None)))
            )
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_WORKTREE"
    );
    assert!(!source.join(".worktrees").exists());
}

#[tokio::test]
async fn plan_rejects_entry_after_unverified_normal_turn() {
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
        .create_session(&workspace.id, "existing work")
        .await
        .unwrap();
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "ordinary turn".into(),
            Arc::new(Provider(Mutex::new(None))),
        )
        .await
        .unwrap();
    assert!(runtime.session_plan_set(&session.id, true).await.is_err());
    assert!(!runtime.session_plan_get(&session.id).await.unwrap());
}

#[tokio::test]
async fn unattended_plan_exit_is_rejected_even_with_auto_approval() {
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
    runtime.set_unattended(true);
    runtime.session_plan_set(&session.id, true).await.unwrap();
    let answer = ModelAnswer {
        calls: vec![ToolCall {
            id: "exit".into(),
            name: "plan_exit".into(),
            arguments: json!({"content":"proposal"}),
        }],
        usage: json!({"prompt_tokens":0,"completion_tokens":0}),
        ..Default::default()
    };
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "plan".into(),
            Arc::new(Provider(Mutex::new(Some(answer)))),
        )
        .await
        .unwrap();
    assert!(runtime.session_plan_get(&session.id).await.unwrap());
}

#[tokio::test]
async fn plan_exit_requires_explicit_selected_approval_and_rejects_switch_while_busy() {
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
    runtime.session_plan_set(&session.id, true).await.unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    for (n, label, source, expected) in [
        (0, "拒绝", AskSource::Selected, true),
        (1, "批准", AskSource::Text, true),
        (2, "批准", AskSource::Selected, false),
    ] {
        let answer = ModelAnswer {
            calls: vec![
                ToolCall {
                    id: format!("exit-{n}"),
                    name: "plan_exit".into(),
                    arguments: json!({"content":"proposal"}),
                },
                ToolCall {
                    id: format!("write-{n}"),
                    name: "write".into(),
                    arguments: json!({"path":"approved.txt","content":"done"}),
                },
            ],
            usage: json!({"prompt_tokens":0,"completion_tokens":0}),
            ..Default::default()
        };
        let task = tokio::spawn(runtime.clone().run_session(
            session.id.clone(),
            "plan".into(),
            Arc::new(CapturingProvider(
                Mutex::new(Some(answer)),
                captured.clone(),
            )),
        ));
        let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(request) = runtime.questions.list().into_iter().next() {
                    break request;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(runtime.session_plan_set(&session.id, false).await.is_err());
        assert!(runtime.session_plan_get(&session.id).await.unwrap());
        assert!(
            !std::path::Path::new(&workspace.replica_path)
                .join("approved.txt")
                .exists()
        );
        runtime
            .questions
            .resolve(
                &request.id,
                AskAnswer {
                    text: label.into(),
                    source,
                },
            )
            .unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(
            runtime.session_plan_get(&session.id).await.unwrap(),
            expected
        );
        assert_eq!(
            std::path::Path::new(&workspace.replica_path)
                .join("approved.txt")
                .exists(),
            !expected
        );
    }
    // 计划模式提示词必须注入模型请求（覆盖由 CapturingProvider 承担）；
    // request.snapshot 已不再落库（写入膨胀源，payload 无消费方）。
    assert!(
        captured
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("当前处于计划模式"))
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(events.iter().all(|e| e.kind != "request.snapshot"));
    assert_eq!(
        events.iter().filter(|e| e.kind == "session.plan").count(),
        2
    );
}

#[tokio::test]
async fn plan_toggle_rejects_busy_session_and_fork_preserves_mode() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let data = temp.path().join("data");
    let runtime = Runtime::open(data.clone()).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "plan")
        .await
        .unwrap();
    let boundary = runtime.session_boundary(&session.id).await;
    assert_eq!(
        runtime
            .session_plan_set(&session.id, true)
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_BUSY"
    );
    drop(boundary);
    runtime.session_plan_set(&session.id, true).await.unwrap();
    let fork = runtime.fork(&session.id).await.unwrap();
    assert!(runtime.session_plan_get(&fork.id).await.unwrap());
    drop(runtime);
    let runtime = Runtime::open(data).await.unwrap();
    assert!(runtime.session_plan_get(&fork.id).await.unwrap());
}

#[tokio::test]
async fn plan_blocks_human_plugin_commands_and_rejects_unverified_plugin_history() {
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
    runtime.session_plan_set(&session.id, true).await.unwrap();
    assert_eq!(
        runtime
            .plugin_command(&session.id, "plugin", "command", json!({}))
            .await
            .unwrap_err()
            .code,
        "PLAN_MODE_READ_ONLY"
    );
    runtime.session_plan_set(&session.id, false).await.unwrap();
    runtime
        .append(&session.id, "plugin.command.started", json!({}))
        .await
        .unwrap();
    assert!(runtime.session_plan_set(&session.id, true).await.is_err());
}

#[tokio::test]
async fn plan_rejects_unverified_long_lived_activity_records() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("data")).await.unwrap();
    let workspace = runtime
        .workspaces
        .create(&source, None, ExecutionMode::Replica)
        .unwrap();
    for kind in [
        "tool.call",
        "job.started",
        "terminal.opened",
        "schedule.created",
    ] {
        let session = runtime
            .store
            .create_session(&workspace.id, kind)
            .await
            .unwrap();
        runtime.append(&session.id, kind, json!({})).await.unwrap();
        assert!(
            runtime.session_plan_set(&session.id, true).await.is_err(),
            "{kind}"
        );
    }
}

#[tokio::test]
async fn plan_cannot_reenter_after_tool_result_outside_plan() {
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
    runtime.session_plan_set(&session.id, true).await.unwrap();
    runtime.session_plan_set(&session.id, false).await.unwrap();
    runtime
        .append(
            &session.id,
            "tool.result",
            json!({"name":"shell","status":"unknown"}),
        )
        .await
        .unwrap();
    assert!(runtime.session_plan_set(&session.id, true).await.is_err());
}

struct Provider(Mutex<Option<ModelAnswer>>);
#[async_trait]
impl ModelProvider for Provider {
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
            .take()
            .unwrap_or_else(|| ModelAnswer {
                usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
                ..Default::default()
            }))
    }
}

/// 记录每轮收到的消息内容拼接，供断言提示词注入（替代已移除的 request.snapshot 落库）。
struct CapturingProvider(Mutex<Option<ModelAnswer>>, Arc<Mutex<Vec<String>>>);
#[async_trait]
impl ModelProvider for CapturingProvider {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.1.lock().unwrap().push(
            messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        Ok(self
            .0
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| ModelAnswer {
                usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
                ..Default::default()
            }))
    }
}

struct DefinitionProbe(Arc<std::sync::atomic::AtomicUsize>);
#[async_trait]
impl ExternalTool for DefinitionProbe {
    fn definition(&self) -> ToolDefinition {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolDefinition {
            name: "definition_probe".into(),
            description: "fixture".into(),
            parameters: json!({"type":"object"}),
        }
    }
    async fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> AppResult<ToolResult> {
        unreachable!()
    }
}

#[tokio::test]
async fn plan_does_not_call_untrusted_tool_definition() {
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
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _registration = runtime
        .register_tool(Arc::new(DefinitionProbe(calls.clone())))
        .unwrap();
    calls.store(0, std::sync::atomic::Ordering::SeqCst);
    runtime.session_plan_set(&session.id, true).await.unwrap();
    runtime
        .clone()
        .run_session(
            session.id,
            "plan".into(),
            Arc::new(Provider(Mutex::new(None))),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

struct UnsafeTool;
#[async_trait]
impl ExternalTool for UnsafeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "unsafe_without_approval".into(),
            description: "fixture".into(),
            parameters: json!({"type":"object"}),
        }
    }
    fn requires_approval(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        context: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> AppResult<ToolResult> {
        std::fs::write(context.root.join("unexpected.txt"), "side effect").unwrap();
        Ok(ToolResult {
            content: "executed".into(),
            is_error: false,
            diff: None,
            escalated: false,
        })
    }
}

#[tokio::test]
async fn plan_denies_auto_write_and_untrusted_approval_exempt_tool_but_allows_read_and_plan_write()
{
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("input.txt"), "safe input").unwrap();
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
    let _registration = runtime.register_tool(Arc::new(UnsafeTool)).unwrap();
    runtime.set_approval_policy(&session.id, Some(ApprovalPolicy::Auto));
    runtime.session_plan_set(&session.id, true).await.unwrap();
    let calls = [
        ("read", "read", json!({"path":"input.txt"})),
        (
            "write",
            "write",
            json!({"path":"new.txt","content":"unsafe"}),
        ),
        ("custom", "unsafe_without_approval", json!({})),
        (
            "shell",
            "shell",
            json!({"command":"echo unsafe","escalate":true}),
        ),
        ("spawn", "agent_spawn", json!({"task":"unsafe"})),
        ("fetch", "web_fetch", json!({"url":"http://127.0.0.1:1/"})),
        ("plan", "plan_write", json!({"content":"read-only plan"})),
    ]
    .into_iter()
    .map(|(id, name, arguments)| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
    .collect();
    let provider = Arc::new(Provider(Mutex::new(Some(ModelAnswer {
        calls,
        usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
        ..Default::default()
    }))));
    runtime
        .clone()
        .run_session(session.id.clone(), "make a plan".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    for id in ["write", "custom", "shell", "spawn", "fetch"] {
        assert!(events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == id
            && event.data["isError"] == true));
    }
    assert!(events.iter().any(|event| event.kind == "tool.result"
        && event.data["callId"] == "read"
        && event.data["isError"] == false));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "plan.state" && event.data["content"] == "read-only plan")
    );
    assert!(
        !std::path::Path::new(&workspace.replica_path)
            .join("new.txt")
            .exists()
    );
    assert!(
        !std::path::Path::new(&workspace.replica_path)
            .join("unexpected.txt")
            .exists()
    );
}

async fn plan_session_fixture(temp: &tempfile::TempDir) -> (Arc<Runtime>, String) {
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
    (runtime, session.id)
}

#[tokio::test]
async fn plan_mode_allows_readonly_shell_without_approval() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id) = plan_session_fixture(&temp).await;
    runtime.set_approval_policy(&session_id, Some(ApprovalPolicy::Auto));
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let calls = [
        ("host", "shell", json!({"command":"hostname"})),
        ("pipe", "shell", json!({"command":"hostname | head -1"})),
    ]
    .into_iter()
    .map(|(id, name, arguments)| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
    .collect();
    let provider = Arc::new(Provider(Mutex::new(Some(ModelAnswer {
        calls,
        usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
        ..Default::default()
    }))));
    runtime
        .clone()
        .run_session(session_id.clone(), "investigate".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    for id in ["host", "pipe"] {
        assert!(events.iter().any(|event| event.kind == "tool.result"
            && event.data["callId"] == id
            && event.data["isError"] == false
            && event.data["status"] == "completed"
            && event.data["escalated"] == false));
    }
    assert!(
        events.iter().all(|event| event.kind != "approval.request"),
        "计划模式只读 shell 不应产生任何审批"
    );
}

#[tokio::test]
async fn plan_mode_denies_writing_shell_and_redirection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("x.txt"), "keep").unwrap();
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
    let session_id = session.id;
    runtime.set_approval_policy(&session_id, Some(ApprovalPolicy::Auto));
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let calls = [
        ("del", "shell", json!({"command":"rm x.txt"})),
        ("redir", "shell", json!({"command":"git log > out.txt"})),
    ]
    .into_iter()
    .map(|(id, name, arguments)| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
    .collect();
    let provider = Arc::new(Provider(Mutex::new(Some(ModelAnswer {
        calls,
        usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
        ..Default::default()
    }))));
    runtime
        .clone()
        .run_session(session_id.clone(), "investigate".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    for id in ["del", "redir"] {
        assert!(
            events.iter().any(|event| event.kind == "tool.result"
                && event.data["callId"] == id
                && event.data["status"] == "denied"),
            "写命令与重定向均应拒绝：{id}"
        );
    }
    assert!(
        std::path::Path::new(&workspace.replica_path)
            .join("x.txt")
            .exists()
    );
    assert!(
        !std::path::Path::new(&workspace.replica_path)
            .join("out.txt")
            .exists()
    );
    assert!(events.iter().all(|event| event.kind != "approval.request"));
}

#[tokio::test]
async fn plan_mode_denies_shell_escalate_requests() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id) = plan_session_fixture(&temp).await;
    runtime.set_approval_policy(&session_id, Some(ApprovalPolicy::Auto));
    runtime.session_plan_set(&session_id, true).await.unwrap();
    let calls = [(
        "esc",
        "shell",
        json!({"command":"hostname","escalate":true}),
    )]
    .into_iter()
    .map(|(id, name, arguments)| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
    .collect();
    let provider = Arc::new(Provider(Mutex::new(Some(ModelAnswer {
        calls,
        usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
        ..Default::default()
    }))));
    runtime
        .clone()
        .run_session(session_id.clone(), "investigate".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    assert!(events.iter().any(|event| event.kind == "tool.result"
        && event.data["callId"] == "esc"
        && event.data["status"] == "denied"));
    assert!(events.iter().all(|event| event.kind != "approval.request"));
}

#[tokio::test]
async fn plan_mode_shell_never_escalates_on_sandbox_denial() {
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session_id) = plan_session_fixture(&temp).await;
    runtime.set_approval_policy(&session_id, Some(ApprovalPolicy::Auto));
    runtime.session_plan_set(&session_id, true).await.unwrap();
    // 读取受 ACL 保护的文件：沙箱内外均失败且命中拒绝关键词，计划模式下不得触发提权重跑审批。
    let calls = [(
        "sam",
        "shell",
        json!({"command":"type C:\\Windows\\System32\\config\\SAM"}),
    )]
    .into_iter()
    .map(|(id, name, arguments)| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
    .collect();
    let provider = Arc::new(Provider(Mutex::new(Some(ModelAnswer {
        calls,
        usage: json!({"prompt_tokens": 0, "completion_tokens": 0}),
        ..Default::default()
    }))));
    runtime
        .clone()
        .run_session(session_id.clone(), "investigate".into(), provider)
        .await
        .unwrap();
    let events = runtime.store.events(&session_id, 0).await.unwrap();
    let result = events
        .iter()
        .find(|event| event.kind == "tool.result" && event.data["callId"] == "sam")
        .expect("应产生 shell 结果事件");
    assert_eq!(
        result.data["status"], "completed",
        "命令被执行（只是失败）而非拒绝"
    );
    assert_eq!(result.data["isError"], true);
    assert_eq!(result.data["escalated"], false, "计划模式不提权重跑");
    assert!(events.iter().all(|event| event.kind != "approval.request"));
}
