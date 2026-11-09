//! Real rust-analyzer smoke test through the full Windows sandbox spawn path.
//! Requires rust-analyzer in PATH; excluded from ordinary runs by design:
//! `cargo test -p harness-runtime --test lsp_smoke -- --ignored --nocapture`
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
        messages: Vec<ModelMessage>,
        _: Vec<ToolDefinition>,
        _: CancellationToken,
        _: DeltaSink,
    ) -> AppResult<ModelAnswer> {
        let _ = messages;
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

async fn tool_result(runtime: &Runtime, session: &str, name: &str) -> SessionEvent {
    for _ in 0..1500 {
        let events = runtime.store.events(session, 0).await.unwrap();
        if let Some(event) = events
            .iter()
            .rev()
            .find(|e| e.kind == "tool.result" && e.data["name"] == name)
        {
            return event.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no tool.result for {name}");
}

#[tokio::test]
#[ignore = "requires rust-analyzer in PATH and exercises the real sandbox spawn"]
async fn real_rust_analyzer_diagnostics_and_hover_inside_sandbox() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("Cargo.toml"),
        "[package]\nname = \"smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"smoke\"\npath = \"main.rs\"\n",
    )
    .unwrap();
    std::fs::write(
        source.join("main.rs"),
        "fn main() {\n    let value: i32 = \"text\";\n    println!(\"{value}\");\n}\n",
    )
    .unwrap();
    let runtime = Runtime::open(temp.path().join("host")).await.unwrap();
    let workspace = runtime.workspaces.create(&source, None).unwrap();
    let session = runtime
        .store
        .create_session(&workspace.id, "smoke")
        .await
        .unwrap();
    let provider = Arc::new(Recorded {
        answers: Mutex::new(VecDeque::from([
            call(
                "c1",
                "lsp_diagnostics",
                json!({"path":"main.rs","wait":true}),
            ),
            call(
                "c2",
                "lsp_diagnostics",
                json!({"path":"main.rs","wait":true}),
            ),
            call(
                "c3",
                "lsp_diagnostics",
                json!({"path":"main.rs","wait":true}),
            ),
            call(
                "c4",
                "lsp_diagnostics",
                json!({"path":"main.rs","wait":true}),
            ),
            call(
                "c5",
                "lsp_hover",
                json!({"path":"main.rs","line":1,"character":5}),
            ),
            call(
                "c6",
                "lsp_hover",
                json!({"path":"main.rs","line":1,"character":5}),
            ),
            ModelAnswer {
                content: "finished".into(),
                ..Default::default()
            },
        ])),
    });
    let task = tokio::spawn(runtime.clone().run_session(
        session.id.clone(),
        "smoke".into(),
        provider.clone(),
    ));
    task.await.unwrap().unwrap();
    // Cold-start analysis (sysroot + build scripts) may lag; wait=true holds until
    // the first push, so a few polls cover slow machines. Must run without concurrent
    // cargo processes: rust-analyzer's cargo check blocks on the package-cache lock.
    let mut diagnostics_text = String::new();
    for _ in 0..3 {
        let event = tool_result(&runtime, &session.id, "lsp_diagnostics").await;
        diagnostics_text = event.data["content"].as_str().unwrap().to_owned();
        let parsed: Value = serde_json::from_str(&diagnostics_text).unwrap();
        let has_entries = parsed.as_array().is_some_and(|files| {
            files
                .iter()
                .any(|f| !f["diagnostics"].as_array().unwrap_or(&vec![]).is_empty())
        });
        if has_entries {
            break;
        }
    }
    println!("diagnostics: {diagnostics_text}");
    let parsed: Value = serde_json::from_str(&diagnostics_text).unwrap();
    assert!(parsed.is_array(), "diagnostics must be a JSON array");
    assert!(
        diagnostics_text.contains("i32") || diagnostics_text.contains("expected"),
        "type mismatch diagnostic expected: {diagnostics_text}"
    );
    let mut hover_text = String::new();
    for _ in 0..3 {
        let event = tool_result(&runtime, &session.id, "lsp_hover").await;
        hover_text = event.data["content"].as_str().unwrap().to_owned();
        if hover_text != "无结果" {
            break;
        }
    }
    println!("hover: {hover_text}");
    assert!(
        hover_text.contains("main"),
        "hover must describe main: {hover_text}"
    );
}
