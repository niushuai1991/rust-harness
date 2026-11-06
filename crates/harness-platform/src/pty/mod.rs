//! Sandboxed pseudo-terminal (ConPTY) sessions for interactive agent commands.
//!
//! The terminal shares the shell sandbox invariants: restricted token, low
//! integrity, capability SID ACLs and a kill-on-close Job Object. Output is
//! decoded at UTF-8 boundaries, ANSI/VT control sequences are stripped for the
//! model and a bounded tail is kept for spill-on-exit persistence.
use harness_contracts::{AppError, AppResult};
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// Cap of unread (not yet taken) output kept for the model reader.
pub const READ_CAP: usize = 64 * 1024;
/// Cap of output buffered between takes before old bytes are dropped.
pub const PENDING_CAP: usize = 64 * 1024;
/// Cap of the post-filter tail kept for spill-on-exit persistence.
pub const TAIL_CAP: usize = 128 * 1024;

/// Streaming ANSI/VT filter tolerant to sequences split across chunk pushes.
#[derive(Debug, Default)]
struct AnsiFilter {
    state: AnsiState,
}
#[derive(Debug, Default, PartialEq)]
enum AnsiState {
    #[default]
    Text,
    Esc,
    Csi,
    /// OSC/DCS/SOS/PM/APC strings, terminated by BEL or ST (ESC \).
    Str,
    /// Saw ESC inside a string, expecting the `\` of ST.
    StrEsc,
}
impl AnsiFilter {
    fn feed(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        for &b in input {
            match self.state {
                AnsiState::Text => match b {
                    0x1b => self.state = AnsiState::Esc,
                    // Keep printable bytes, tab/newline/carriage return; drop other C0/DEL.
                    0x09 | 0x0a | 0x0d | 0x20..=0x7e | 0x80..=0xff => out.push(b),
                    _ => {}
                },
                AnsiState::Esc => match b {
                    b'[' => self.state = AnsiState::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => self.state = AnsiState::Str,
                    0x1b => self.state = AnsiState::Esc,
                    // Charset/attribute introducers take one designating byte.
                    b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.state = AnsiState::Esc,
                    _ => self.state = AnsiState::Text,
                },
                AnsiState::Csi => {
                    // Parameter/intermediate bytes until the final byte 0x40..=0x7e.
                    if (0x40..=0x7e).contains(&b) {
                        self.state = AnsiState::Text;
                    }
                }
                AnsiState::Str => match b {
                    0x07 => self.state = AnsiState::Text,
                    0x1b => self.state = AnsiState::StrEsc,
                    _ => {}
                },
                AnsiState::StrEsc => match b {
                    b'\\' => self.state = AnsiState::Text,
                    0x1b => self.state = AnsiState::StrEsc,
                    _ => self.state = AnsiState::Str,
                },
            }
        }
        out
    }
}

/// Remove ANSI/VT escape sequences from one complete text chunk.
#[cfg(test)]
pub(crate) fn strip_ansi(input: &str) -> String {
    String::from_utf8_lossy(&AnsiFilter::default().feed(input.as_bytes())).into_owned()
}

/// Accumulates terminal output: filtered unread pending plus a capped spill tail.
#[derive(Debug, Default)]
pub struct OutputBuffer {
    filter: AnsiFilter,
    pending: Vec<u8>,
    tail: Vec<u8>,
    pending_truncated: bool,
    tail_truncated: bool,
}
impl OutputBuffer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Feed raw console bytes; escape sequences split across pushes are handled.
    pub fn push(&mut self, bytes: &[u8]) {
        let clean = self.filter.feed(bytes);
        if clean.is_empty() {
            return;
        }
        self.pending.extend_from_slice(&clean);
        if self.pending.len() > PENDING_CAP {
            self.pending.drain(..self.pending.len() - PENDING_CAP);
            self.pending_truncated = true;
        }
        self.tail.extend_from_slice(&clean);
        if self.tail.len() > TAIL_CAP {
            self.tail.drain(..self.tail.len() - TAIL_CAP);
            self.tail_truncated = true;
        }
    }
    /// Filtered bytes waiting to be taken.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
    /// Drain unread output as model text: CRLF normalized, lossy UTF-8,
    /// capped to `max` bytes at a character boundary with a truncation marker.
    pub fn take(&mut self, max: usize) -> String {
        let mut bytes = normalize_newlines(&std::mem::take(&mut self.pending));
        let dropped = std::mem::take(&mut self.pending_truncated);
        if bytes.len() > max || dropped {
            if bytes.len() > max {
                let mut cut = max;
                while cut > 0 && (bytes[cut] & 0xC0) == 0x80 {
                    cut -= 1;
                }
                bytes.truncate(cut);
            }
            bytes.extend_from_slice("\n[输出超出单次读取上限，已截断]".as_bytes());
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
    /// Spill tail for persistence: (text, truncated-flag).
    pub fn tail_text(&self) -> (String, bool) {
        (
            String::from_utf8_lossy(&normalize_newlines(&self.tail)).into_owned(),
            self.tail_truncated,
        )
    }
}
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\n');
            i += if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                2
            } else {
                1
            };
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}
type ResizeFn = Box<dyn Fn(u16, u16) -> AppResult<()> + Send + Sync>;
/// One live sandboxed pseudo-terminal running an interactive shell in the replica.
pub struct SandboxedPty {
    input: tokio::fs::File,
    buffer: std::sync::Arc<std::sync::Mutex<OutputBuffer>>,
    exit: tokio::sync::oneshot::Receiver<i32>,
    cancel: CancellationToken,
    resize: ResizeFn,
    _guard: Box<dyn Send>,
}
pub struct PtyService;
impl PtyService {
    /// Spawn an interactive sandboxed shell (pwsh 7, falling back to Windows
    /// PowerShell) rooted at the managed replica, attached to a ConPTY.
    pub async fn spawn(root: &Path, cancel: CancellationToken) -> AppResult<SandboxedPty> {
        #[cfg(windows)]
        {
            windows::spawn(root, cancel)
        }
        #[cfg(not(windows))]
        {
            let _ = (root, cancel);
            Err(AppError::new(
                "UNSUPPORTED",
                "PTY isolation is Windows only",
            ))
        }
    }
}
impl SandboxedPty {
    /// Send keystrokes to the terminal. `\n` is normalized to `\r` because the
    /// Windows console input processor treats CR as Enter and often ignores LF.
    pub async fn write_input(&mut self, input: &str) -> AppResult<()> {
        use tokio::io::AsyncWriteExt;
        if input.contains('\0') {
            return Err(AppError::new("INVALID_ARGUMENT", "NUL in terminal input"));
        }
        let normalized = input.replace("\r\n", "\r").replace('\n', "\r");
        self.input
            .write_all(normalized.as_bytes())
            .await
            .map_err(crate::io)?;
        self.input.flush().await.map_err(crate::io)
    }
    /// Incremental unread output; waits up to `yield_ms` when nothing is pending.
    pub async fn read_output(&mut self, yield_ms: u64) -> AppResult<String> {
        let budget =
            std::time::Instant::now() + std::time::Duration::from_millis(yield_ms.min(30_000));
        loop {
            if self.buffer.lock().unwrap().pending_len() > 0 || std::time::Instant::now() >= budget
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        Ok(self.buffer.lock().unwrap().take(READ_CAP))
    }
    /// Process exit code; None if the process outlived the wait (still running).
    pub async fn exit_code(&mut self) -> Option<i32> {
        (&mut self.exit).await.ok()
    }
    /// Spill tail for persistence: (text, truncated-flag).
    pub fn spill(&self) -> (String, bool) {
        self.buffer.lock().unwrap().tail_text()
    }
    /// Kill the whole process tree via the Job Object.
    pub fn terminate(&self) {
        self.cancel.cancel();
    }
    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        (self.resize)(cols, rows)
    }
}
#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips_csi_osc_and_keeps_text() {
        assert_eq!(strip_ansi("\x1b[31m红\x1b[0m色\x1b]0;标题\x07!"), "红色!");
    }
    #[test]
    fn strips_two_char_escapes_and_str_sequences() {
        assert_eq!(strip_ansi("\x1bP$q\x1b\\x\x1b(B\n"), "x\n");
    }
    #[test]
    fn filter_survives_chunk_split() {
        let mut b = OutputBuffer::new();
        b.push(b"a\x1b[3");
        b.push(b"1mb");
        assert_eq!(b.take(READ_CAP), "ab");
    }
    #[test]
    fn take_caps_at_char_boundary_with_marker() {
        let mut b = OutputBuffer::new();
        b.push("中文测试".repeat(20_000).as_bytes());
        let text = b.take(64);
        assert!(text.contains("[输出超出单次读取上限，已截断]"));
        assert!(text.len() < 64 + "[输出超出单次读取上限，已截断]".len() + 4);
        assert_eq!(b.take(READ_CAP), "");
    }
    #[test]
    fn pending_cap_marks_truncation() {
        let mut b = OutputBuffer::new();
        b.push(&[b'x'; PENDING_CAP + 1]);
        assert!(b.take(READ_CAP).contains("[输出超出单次读取上限，已截断]"));
    }
    #[test]
    fn tail_caps_and_reports_truncated() {
        let mut b = OutputBuffer::new();
        b.push(&[b'y'; TAIL_CAP + 4096]);
        let (text, truncated) = b.tail_text();
        assert!(truncated);
        assert!(text.len() <= TAIL_CAP);
    }
    #[test]
    fn normalizes_crlf_and_bare_cr() {
        let mut b = OutputBuffer::new();
        b.push(b"a\r\nb\rc");
        assert_eq!(b.take(READ_CAP), "a\nb\nc");
    }
    #[test]
    fn keeps_utf8_multibyte_through_filter() {
        assert_eq!(strip_ansi("\x1b[2J你好\x1b[m"), "你好");
    }
}
