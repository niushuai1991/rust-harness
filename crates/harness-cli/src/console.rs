//! 事件流渲染器：human（正文 stdout、过程 stderr、request 关联去重）与 json（JSONL 契约）。
use crate::diffview::ColorChoice;
use harness_contracts::{ApprovalRequest, SessionEvent};
use serde_json::json;
use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
};

/// 测试与调用方共享的缓冲输出。
#[derive(Clone, Default)]
pub struct SharedBuf(pub Arc<Mutex<Vec<u8>>>);
impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl SharedBuf {
    pub fn string(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[derive(Default, Clone)]
struct StreamState {
    /// 当前 request.snapshot 的 id
    current_request: Option<String>,
    /// 当前请求是否流出过正文 Delta（reasoning=false）
    streamed_content: bool,
}

/// 终行 result 契约（JSON 模式 stdout 最后一行）。
#[derive(Debug, Clone)]
pub struct RunResult {
    pub message_id: String,
    pub run_id: Option<String>,
    pub session_id: String,
    pub workspace_id: String,
    /// completed | error | cancelled
    pub status: String,
    /// 0 | 1 | 130
    pub exit_code: i32,
    pub final_message: String,
    pub changed_files: usize,
}

pub struct Console {
    json: bool,
    parent: String,
    out: Box<dyn Write>,
    err: Box<dyn Write>,
    states: HashMap<String, StreamState>,
    child_buffers: HashMap<String, String>,
    force_full: bool,
    last_assistant: String,
    #[allow(dead_code)]
    color: ColorChoice,
}

impl Console {
    pub fn new(json: bool, parent: &str, color: ColorChoice) -> Self {
        Self::with_writers(
            json,
            parent,
            color,
            Box::new(std::io::stdout()),
            Box::new(std::io::stderr()),
        )
    }

    pub fn with_writers(
        json: bool,
        parent: &str,
        color: ColorChoice,
        out: Box<dyn Write>,
        err: Box<dyn Write>,
    ) -> Self {
        Self {
            json,
            parent: parent.to_owned(),
            out,
            err,
            states: HashMap::new(),
            child_buffers: HashMap::new(),
            force_full: false,
            last_assistant: String::new(),
            color,
        }
    }

    fn child_prefix(&self, session: &str) -> String {
        if session == self.parent {
            String::new()
        } else {
            format!("[子代理 {}] ", &session[..session.len().min(8)])
        }
    }

    fn state(&mut self, session: &str) -> &mut StreamState {
        self.states.entry(session.to_owned()).or_default()
    }

    /// 一条作用域内的 Durable 事件。
    pub fn durable(&mut self, event: &SessionEvent) -> std::io::Result<()> {
        if self.json {
            let line = json!({"type":"event","event":event});
            writeln!(self.out, "{line}")?;
            self.out.flush()?;
        }
        if event.kind == "request.snapshot" {
            let id = event.data["id"].as_str().unwrap_or_default().to_owned();
            let state = self.state(&event.session_id);
            state.current_request = Some(id);
            state.streamed_content = false;
            return Ok(());
        }
        if event.kind == "assistant.message" {
            let content = event.data["content"].as_str().unwrap_or_default();
            if event.session_id == self.parent {
                let force_full = self.force_full;
                let suppress = {
                    let state = self.state(&event.session_id);
                    let suppress = state.streamed_content && !force_full;
                    state.current_request = None;
                    state.streamed_content = false;
                    suppress
                };
                self.force_full = false;
                self.last_assistant = content.to_owned();
                if !self.json && !suppress && !content.is_empty() {
                    writeln!(self.out, "{content}")?;
                    self.out.flush()?;
                }
            } else if !self.json {
                // 子代理：缓冲的增量一次性带前缀输出，避免逐块前缀噪音
                let buffered = self.child_buffers.remove(&event.session_id);
                let text = match buffered {
                    Some(text) if !text.is_empty() => text,
                    _ => content.to_owned(),
                };
                if !text.is_empty() {
                    writeln!(self.err, "{}{}", self.child_prefix(&event.session_id), text)?;
                    self.err.flush()?;
                }
            }
            return Ok(());
        }
        let prefix = self.child_prefix(&event.session_id);
        match event.kind.as_str() {
            "tool.call" => {
                let name = event.data["name"].as_str().unwrap_or_default();
                writeln!(self.err, "{prefix}→ {name} {}", event.data["arguments"])?;
            }
            "tool.result" => {
                if event.data["isError"] == true {
                    let content = event.data["content"].as_str().unwrap_or_default();
                    let snippet: String = content.chars().take(300).collect();
                    writeln!(self.err, "{prefix}✗ {}", snippet.trim())?;
                }
            }
            "inbox.cancelled" => {
                writeln!(self.err, "{prefix}消息已在排队时取消")?;
            }
            _ => {}
        }
        if !self.json {
            self.err.flush()?;
        }
        Ok(())
    }

    /// 一条作用域内的流式增量。
    pub fn delta(
        &mut self,
        session: &str,
        request_id: &str,
        index: u32,
        text: &str,
        reasoning: bool,
    ) -> std::io::Result<()> {
        if self.json {
            let line = json!({"type":"delta","sessionId":session,"requestId":request_id,"index":index,"text":text,"reasoning":reasoning});
            writeln!(self.out, "{line}")?;
            self.out.flush()?;
        }
        if !reasoning {
            let state = self.state(session);
            if state.current_request.as_deref() == Some(request_id) {
                state.streamed_content = true;
            }
        }
        if session != self.parent {
            if !self.json {
                self.child_buffers
                    .entry(session.to_owned())
                    .or_default()
                    .push_str(text);
            }
        } else if !self.json {
            // human 模式：正文流式输出到 stdout，reasoning 到 stderr；json 模式仅事件行
            if reasoning {
                write!(self.err, "{text}")?;
                self.err.flush()?;
            } else {
                write!(self.out, "{text}")?;
                self.out.flush()?;
            }
        }
        Ok(())
    }

    /// Lagged 降级：Delta/Status 不可回补，仅声明缺口并强制补打后续完整正文。
    pub fn gap(&mut self, session: &str, from: u64, to: u64) -> std::io::Result<()> {
        self.force_full = true;
        if self.json {
            let line = json!({"type":"event_gap","sessionId":session,"fromSeq":from,"toSeq":to});
            writeln!(self.out, "{line}")?;
            self.out.flush()?;
        } else {
            writeln!(self.err, "⚠ 事件流缺口 {from}..{to}，正文将以完整消息补发")?;
            self.err.flush()?;
        }
        Ok(())
    }

    /// 审批请求提示（stderr，保证 stdout 纯净）。
    pub fn approval(&mut self, request: &ApprovalRequest) -> std::io::Result<()> {
        writeln!(
            self.err,
            "\n待审批 {}（会话 {}）\n  参数: {}",
            request.tool, request.session_id, request.arguments
        )?;
        write!(self.err, "  允许执行？[y/N] ")?;
        self.err.flush()
    }

    /// 人类可读提示（一律 stderr；JSON 模式 stdout 仅事件与 result 行）。
    pub fn note(&mut self, message: &str) -> std::io::Result<()> {
        writeln!(self.err, "{message}")?;
        self.err.flush()
    }

    /// 终行 result：JSON 模式写 stdout；human 模式写摘要到 stderr。
    pub fn result(&mut self, result: &RunResult) -> std::io::Result<()> {
        if self.json {
            let line = json!({
                "type":"result",
                "messageId":result.message_id,
                "runId":result.run_id,
                "sessionId":result.session_id,
                "workspaceId":result.workspace_id,
                "status":result.status,
                "exitCode":result.exit_code,
                "finalMessage":result.final_message,
                "changedFiles":result.changed_files,
            });
            writeln!(self.out, "{line}")?;
            self.out.flush()?;
        } else {
            match result.status.as_str() {
                "completed" => {
                    let message = result.final_message.trim();
                    if !message.is_empty() && message != self.last_assistant.as_str() {
                        writeln!(self.err, "{message}")?;
                    }
                    writeln!(
                        self.err,
                        "回合完成，副本改动 {} 个文件。",
                        result.changed_files
                    )?;
                }
                "cancelled" => writeln!(self.err, "回合已取消。")?,
                _ => writeln!(self.err, "回合失败。")?,
            }
            if result.changed_files > 0 {
                writeln!(
                    self.err,
                    "审阅同步：harness diff {} → harness apply {}",
                    result.workspace_id, result.workspace_id
                )?;
            }
            self.err.flush()?;
        }
        Ok(())
    }

    pub fn last_assistant(&self) -> &str {
        &self.last_assistant
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diffview::ColorChoice;

    fn event(session: &str, kind: &str, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            id: "e".into(),
            session_id: session.into(),
            seq: "1".into(),
            kind: kind.into(),
            data,
            created_at: String::new(),
        }
    }

    fn console() -> (Console, SharedBuf, SharedBuf) {
        let out = SharedBuf::default();
        let err = SharedBuf::default();
        let console = Console::with_writers(
            false,
            "s1",
            ColorChoice::Never,
            Box::new(out.clone()),
            Box::new(err.clone()),
        );
        (console, out, err)
    }

    #[test]
    fn human_suppresses_assistant_message_after_streamed_content_delta() {
        let (mut console, out, _err) = console();
        console
            .durable(&event(
                "s1",
                "request.snapshot",
                json!({"id":"r1","step":1}),
            ))
            .unwrap();
        console.delta("s1", "r1", 0, "你", false).unwrap();
        console.delta("s1", "r1", 1, "好", false).unwrap();
        assert_eq!(out.string(), "你好");
        console
            .durable(&event("s1", "assistant.message", json!({"content":"你好"})))
            .unwrap();
        assert_eq!(out.string(), "你好");
        assert_eq!(console.last_assistant(), "你好");
    }

    #[test]
    fn human_prints_full_message_when_only_reasoning_delta_seen() {
        let (mut console, out, _err) = console();
        console
            .durable(&event(
                "s1",
                "request.snapshot",
                json!({"id":"r1","step":1}),
            ))
            .unwrap();
        console.delta("s1", "r1", 0, "思考中", true).unwrap();
        console
            .durable(&event(
                "s1",
                "assistant.message",
                json!({"content":"最终答案"}),
            ))
            .unwrap();
        assert_eq!(out.string().trim_end(), "最终答案");
    }

    #[test]
    fn gap_forces_next_assistant_message_full_text() {
        let (mut console, out, err) = console();
        console
            .durable(&event(
                "s1",
                "request.snapshot",
                json!({"id":"r1","step":1}),
            ))
            .unwrap();
        console.delta("s1", "r1", 0, "部", false).unwrap();
        console.gap("s1", 5, 9).unwrap();
        console
            .durable(&event(
                "s1",
                "assistant.message",
                json!({"content":"完整正文"}),
            ))
            .unwrap();
        assert!(err.string().contains("事件流缺口"));
        assert!(out.string().trim_end().ends_with("完整正文"));
    }

    #[test]
    fn child_output_goes_to_stderr_with_prefix_once() {
        let (mut console, out, err) = console();
        console
            .durable(&event(
                "c1",
                "request.snapshot",
                json!({"id":"r1","step":1}),
            ))
            .unwrap();
        console.delta("c1", "r1", 0, "子代理", false).unwrap();
        console.delta("c1", "r1", 1, "输出", false).unwrap();
        console
            .durable(&event(
                "c1",
                "tool.call",
                json!({"name":"read","arguments":{"path":"a"}}),
            ))
            .unwrap();
        console
            .durable(&event(
                "c1",
                "assistant.message",
                json!({"content":"子代理输出"}),
            ))
            .unwrap();
        assert!(out.string().is_empty(), "子代理正文不得进入 stdout");
        let text = err.string();
        assert!(text.contains("→ read"));
        // 缓冲增量一次性输出一次，且不重复打印 assistant 全文
        assert_eq!(text.matches("子代理输出").count(), 1);
        assert!(text.contains("[子代理 c1] 子代理输出"));
    }

    #[test]
    fn json_lines_are_parseable_and_stdout_stays_pure() {
        let out = SharedBuf::default();
        let err = SharedBuf::default();
        let mut console = Console::with_writers(
            true,
            "s1",
            ColorChoice::Never,
            Box::new(out.clone()),
            Box::new(err.clone()),
        );
        console.delta("s1", "r1", 0, "你好", false).unwrap();
        console
            .durable(&event("s1", "assistant.message", json!({"content":"你好"})))
            .unwrap();
        console.gap("s1", 3, 4).unwrap();
        console
            .result(&RunResult {
                message_id: "m1".into(),
                run_id: Some("r9".into()),
                session_id: "s1".into(),
                workspace_id: "w1".into(),
                status: "completed".into(),
                exit_code: 0,
                final_message: "你好".into(),
                changed_files: 2,
            })
            .unwrap();
        for line in out.string().lines() {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("每行必须可独立解析: {e}: {line}"));
            assert!(value["type"].is_string());
        }
        let lines: Vec<String> = out.string().lines().map(str::to_owned).collect();
        assert_eq!(lines.len(), 4);
        let result: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(result["type"], "result");
        assert_eq!(result["exitCode"], 0);
        assert_eq!(result["changedFiles"], 2);
        assert!(err.string().is_empty(), "JSON 模式 stdout 必须纯净");
    }
}
