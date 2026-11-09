use async_trait::async_trait;
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

struct Recorded {
    answers: Mutex<VecDeque<ModelAnswer>>,
    requests: Mutex<Vec<Vec<ModelMessage>>>,
}
#[async_trait]
impl ModelProvider for Recorded {
    async fn complete(
        &self,
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        self.requests.lock().unwrap().push(messages);
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

#[tokio::test]
async fn approved_edit_stays_in_replica_until_explicit_sync() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), "old").unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "test")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "read", json!({"path":"a.txt"})),
            call("c2", "write", json!({"path":"a.txt","content":"new"})),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "change".into(),
        provider.clone(),
    ));
    let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let list = runtime.approvals.list();
            if let Some(r) = list.first() {
                break r.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending.tool, "write");
    runtime.approvals.resolve(&pending.id, true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&workspace.replica_path).join("a.txt"))
            .unwrap(),
        "new"
    );
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "tool.result").count(), 2);
    assert_eq!(events.last().unwrap().data["status"], "completed");
    assert!(
        provider.requests.lock().unwrap()[2]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content == "old")
    );
    runtime
        .workspaces
        .apply(&workspace.id, &["a.txt".into()])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(source.join("a.txt")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn denied_write_is_logged_without_mutating_the_replica() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "deny")
        .await
        .unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call("c1", "write", json!({"path":"a.txt","content":"denied"})),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "try write".into(),
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
    assert!(
        !std::path::Path::new(&workspace.replica_path)
            .join("a.txt")
            .exists()
    );
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

#[tokio::test]
async fn duplicate_tool_ids_are_rejected_before_settling_assistant_history() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let w = runtime.workspaces.create(&source, None).unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "duplicate")
        .await
        .unwrap();
    let c = ToolCall {
        id: "same".into(),
        name: "list_files".into(),
        arguments: json!({}),
    };
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            calls: vec![c.clone(), c],
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    assert!(
        runtime
            .clone()
            .run_session(s.id.clone(), "test".into(), model)
            .await
            .is_err()
    );
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    assert!(!events.iter().any(|e| e.kind == "assistant.message"));
    assert!(events.iter().any(|e| e.kind == "assistant.attempt"));
}

struct WritesThenWaits;
#[async_trait]
impl harness_runtime::ExternalTool for WritesThenWaits {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "test_side_effect".into(),
            description: "test".into(),
            parameters: json!({"type":"object"}),
        }
    }
    fn requires_approval(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        context: harness_runtime::ToolContext,
        _: Value,
        cancel: CancellationToken,
    ) -> AppResult<ToolResult> {
        std::fs::write(context.root.join("effect.txt"), "already happened").unwrap();
        cancel.cancelled().await;
        Err(AppError::new("CANCELLED", "cancelled after writing"))
    }
}

#[tokio::test]
async fn cancelling_after_side_effect_records_unknown_outcome_and_releases_lease() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let w = runtime.workspaces.create(&source, None).unwrap();
    let s = runtime.store.create_session(&w.id, "cancel").await.unwrap();
    let _guard = runtime.register_tool(Arc::new(WritesThenWaits)).unwrap();
    let model = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([call("c1", "test_side_effect", json!({}))])),
        requests: Mutex::new(vec![]),
    });
    let task = tokio::spawn(
        runtime
            .clone()
            .run_session(s.id.clone(), "test".into(), model),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !std::path::Path::new(&w.replica_path)
            .join("effect.txt")
            .exists()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    runtime.cancel(&s.id);
    assert!(task.await.unwrap().is_err());
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "tool.result" && e.data["status"] == "unknown")
    );
    assert!(runtime.workspaces.diff(&w.id).is_ok());
}

#[tokio::test]
async fn attachments_enter_model_history_and_spilled_output_is_readable_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let huge = "完整输出".repeat(10000);
    std::fs::write(source.join("large.txt"), &huge).unwrap();
    let data = dir.path().join("host");
    let runtime = Runtime::open(data.clone()).await.unwrap();
    let w = runtime.workspaces.create(&source, None).unwrap();
    let s = runtime
        .store
        .create_session(&w.id, "artifacts")
        .await
        .unwrap();
    let private = dir.path().join("private-host-path.txt");
    std::fs::write(&private, "附件说明").unwrap();
    let meta = runtime.artifact_import(&s.id, &private).await.unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "attached",
                "read_artifact",
                json!({"hash":meta.hash,"offset":0,"length":32768}),
            ),
            call("large", "read", json!({"path":"large.txt"})),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session_with_attachments(
            s.id.clone(),
            "检查附件".into(),
            provider.clone(),
            vec![meta.hash.clone()],
        )
        .await
        .unwrap();
    {
        let requests = provider.requests.lock().unwrap();
        let history = serde_json::to_string(&requests[0]).unwrap();
        assert!(history.contains(&meta.hash));
        assert!(history.contains("private-host-path.txt"));
        assert!(!history.contains(&private.to_string_lossy().replace('\\', "\\\\")));
        assert!(
            requests[1]
                .iter()
                .any(|m| m.role == MessageRole::Tool && m.content.contains("附件说明"))
        );
    }
    let events = runtime.store.events(&s.id, 0).await.unwrap();
    let artifact = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.data["callId"] == "large")
        .unwrap()
        .data["artifact"]
        .clone();
    assert_eq!(artifact["size"], huge.len());
    let hash = artifact["hash"].as_str().unwrap().to_owned();
    drop(runtime);
    let runtime = Runtime::open(data).await.unwrap();
    let chunk = runtime.artifact_read(&s.id, &hash, 0, 32768).await.unwrap();
    assert!(huge.starts_with(&chunk.content));
    assert!(chunk.next_offset.is_some());
    let replay_provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "spill-read",
                "read_artifact",
                json!({"hash":hash,"offset":0,"length":32768}),
            ),
            ModelAnswer::default(),
        ])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            s.id.clone(),
            "继续读取完整输出".into(),
            replay_provider.clone(),
        )
        .await
        .unwrap();
    assert!(
        replay_provider.requests.lock().unwrap()[1]
            .iter()
            .any(|m| m.role == MessageRole::Tool && m.content.contains("完整输出"))
    );
    let other = runtime.store.create_session(&w.id, "other").await.unwrap();
    assert!(
        runtime
            .artifact_read(&other.id, &hash, 0, 32768)
            .await
            .is_err()
    );
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer::default()])),
        requests: Mutex::new(vec![]),
    });
    assert!(
        runtime
            .clone()
            .run_session_with_attachments(
                other.id.clone(),
                "steal".into(),
                provider,
                vec![meta.hash]
            )
            .await
            .is_err()
    );
    assert!(
        !runtime
            .store
            .events(&other.id, 0)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == "user.message")
    );
}

#[tokio::test]
async fn skills_catalog_enters_system_prompt_and_mention_injects_body() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let skill_dir = source.join(".agents").join("skills").join("review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: review\ndescription: 代码审查流程\n---\n审查正文内容",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "skills")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer {
            content: "ok".into(),
            ..Default::default()
        }])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "请 $review 审查，注意 $PATH 是环境变量".into(),
            provider.clone(),
        )
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    let loaded: Vec<_> = events.iter().filter(|e| e.kind == "skill.loaded").collect();
    assert_eq!(loaded.len(), 1, "$PATH 不在技能名单不注入");
    assert_eq!(loaded[0].data["name"], "review");
    assert_eq!(
        loaded[0].data["content"], "审查正文内容",
        "正文剥离 frontmatter"
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let system = requests[0]
        .iter()
        .find(|m| m.role == MessageRole::System)
        .unwrap();
    assert!(system.content.contains("## 可用技能"));
    assert!(system.content.contains("- review: 代码审查流程"));
    assert!(system.content.contains("使用规则"));
    assert!(
        !system.content.contains("可按需读取的技能"),
        "旧裸路径清单已被 catalog 取代"
    );
    assert!(
        requests[0]
            .iter()
            .any(|m| m.role == MessageRole::User && m.content.contains("<skill name=\"review\""))
    );
}

#[tokio::test]
async fn disabled_skill_is_absent_from_catalog_and_mention() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let skill_dir = source
        .join(".agents")
        .join("skills")
        .join("zskills-fixture");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\ndescription: 代码审查流程\n---\n审查正文内容",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    runtime
        .skills
        .set_enabled(".agents/skills/zskills-fixture/SKILL.md", false)
        .await
        .unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "disabled")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([ModelAnswer::default()])),
        requests: Mutex::new(vec![]),
    });
    runtime
        .clone()
        .run_session(
            session.id.clone(),
            "请 $zskills-fixture 审查".into(),
            provider.clone(),
        )
        .await
        .unwrap();
    let events = runtime.store.events(&session.id, 0).await.unwrap();
    assert!(!events.iter().any(|e| e.kind == "skill.loaded"));
    {
        let requests = provider.requests.lock().unwrap();
        let system = &requests[0]
            .iter()
            .find(|m| m.role == MessageRole::System)
            .unwrap()
            .content;
        assert!(
            !system.contains("zskills-fixture"),
            "禁用的技能不进 catalog（用户级技能可能存在，不整段断言）"
        );
        assert!(!requests[0].iter().any(|m| m.role == MessageRole::User
            && m.content.contains("<skill name=\"zskills-fixture\"")));
    }
    let list = runtime
        .skills
        .list(std::path::Path::new(&workspace.replica_path))
        .await
        .unwrap();
    let fixture = list
        .iter()
        .find(|s| s.path == ".agents/skills/zskills-fixture/SKILL.md")
        .expect("fixture 技能在目录中");
    assert!(!fixture.enabled);
    assert!(skill_path.exists(), "禁用不动文件");
}
