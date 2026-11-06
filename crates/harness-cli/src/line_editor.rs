//! chat REPL 的单行行编辑器：逻辑状态机（可单测）+ crossterm 渲染薄层。
//!
//! raw mode 仅在 [`read_line`] 等待输入期间启用，返回前恢复，保证回合流式输出
//! 与审批 y/n 读取（`run::read_confirmation`）共用同一 stdin 的串行约定不受影响。

use crate::slash::{SlashSpec, filter_slash_commands};
use crossterm::{
    cursor::{MoveTo, position},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode, size},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

/// 弹窗最多渲染的候选行数。
const MAX_POPUP_ROWS: usize = 8;
/// 选中项前缀。
const SELECTED_MARK: &str = "› ";
/// 未选中项前缀。
const ITEM_MARK: &str = "  ";

/// 字符显示宽度（CJK 按双宽计），用于光标定位。
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// 编辑器可感知的按键动作（渲染层负责把 crossterm 事件翻译成它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Tab,
    Esc,
    Enter,
    CtrlC,
    CtrlD,
    Other,
}

/// `handle` 的终结结果；`None` 表示继续编辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Enter 提交当前行（弹窗激活时已应用补全）。
    Submitted,
    /// 行为空时的 Ctrl+C，由调用方决定退出。
    Interrupt,
    /// 空行上的 Ctrl+D（EOF）。
    Eof,
}

/// 单行编辑器状态机：缓冲、光标、历史与补全弹窗。
#[derive(Debug, Default)]
pub struct Editor {
    buffer: String,
    /// 字节偏移，始终落在字符边界上。
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    /// 开始翻历史前的草稿，`Down` 回到末尾时恢复。
    draft: Option<String>,
    candidates: Vec<&'static SlashSpec>,
    selected: usize,
    popup_open: bool,
    /// Esc 关闭弹窗时的首 token；同一 token 内不再自动弹出。
    dismissed: Option<String>,
}

impl Editor {
    pub fn new(history: Vec<String>) -> Self {
        Self {
            history,
            ..Self::default()
        }
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// 光标字节偏移。
    pub fn cursor_bytes(&self) -> usize {
        self.cursor
    }

    /// 光标显示宽度偏移（CJK 双宽）。
    pub fn cursor_width(&self) -> usize {
        display_width(&self.buffer[..self.cursor])
    }

    pub fn popup_open(&self) -> bool {
        self.popup_open
    }

    pub fn popup_candidates(&self) -> &[&'static SlashSpec] {
        &self.candidates
    }

    pub fn popup_selected(&self) -> usize {
        self.selected
    }

    /// 处理一个按键动作。
    pub fn handle(&mut self, action: KeyAction) -> Option<Outcome> {
        match action {
            KeyAction::Char(c) => {
                self.insert_char(c);
                None
            }
            KeyAction::Backspace => {
                self.delete_before();
                None
            }
            KeyAction::Delete => {
                self.delete_at();
                None
            }
            KeyAction::Left => {
                self.move_left();
                None
            }
            KeyAction::Right => {
                self.move_right();
                None
            }
            KeyAction::Home => {
                self.cursor = 0;
                self.sync_popup();
                None
            }
            KeyAction::End => {
                self.cursor = self.buffer.len();
                self.sync_popup();
                None
            }
            KeyAction::Up => self.move_up(),
            KeyAction::Down => self.move_down(),
            KeyAction::Tab => {
                self.complete_selected();
                None
            }
            KeyAction::Esc => {
                self.dismiss_popup();
                None
            }
            KeyAction::Enter => self.submit(),
            KeyAction::CtrlC => self.interrupt_or_clear(),
            KeyAction::CtrlD => self.eof_if_empty(),
            KeyAction::Other => None,
        }
    }

    fn insert_char(&mut self, c: char) {
        self.leave_history();
        let mut start = self.cursor.min(self.buffer.len());
        while start > 0 && !self.buffer.is_char_boundary(start) {
            start -= 1;
        }
        self.buffer.insert(start, c);
        self.cursor = start + c.len_utf8();
        self.sync_popup();
    }

    fn delete_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut start = self.cursor;
        while start > 0 && !self.buffer.is_char_boundary(start) {
            start -= 1;
        }
        let mut prev = start - 1;
        while prev > 0 && !self.buffer.is_char_boundary(prev) {
            prev -= 1;
        }
        self.buffer.replace_range(prev..start, "");
        self.cursor = prev;
        self.leave_history();
        self.sync_popup();
    }

    fn delete_at(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let mut end = self.cursor + 1;
        while end < self.buffer.len() && !self.buffer.is_char_boundary(end) {
            end += 1;
        }
        self.buffer.replace_range(self.cursor..end, "");
        self.leave_history();
        self.sync_popup();
    }

    fn move_left(&mut self) {
        let mut prev = self.cursor.saturating_sub(1);
        while prev > 0 && !self.buffer.is_char_boundary(prev) {
            prev -= 1;
        }
        self.cursor = prev;
        self.sync_popup();
    }

    fn move_right(&mut self) {
        let mut next = (self.cursor + 1).min(self.buffer.len());
        while next < self.buffer.len() && !self.buffer.is_char_boundary(next) {
            next += 1;
        }
        self.cursor = next;
        self.sync_popup();
    }

    fn move_up(&mut self) -> Option<Outcome> {
        if self.popup_open {
            if self.candidates.is_empty() {
                return None;
            }
            self.selected = if self.selected == 0 {
                self.candidates.len() - 1
            } else {
                self.selected - 1
            };
            return None;
        }
        if self.history.is_empty() {
            return None;
        }
        let index = match self.history_index {
            None => {
                self.draft = Some(self.buffer.clone());
                self.history.len() - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.history_index = Some(index);
        self.buffer = self.history[index].clone();
        self.cursor = self.buffer.len();
        self.sync_popup();
        None
    }

    fn move_down(&mut self) -> Option<Outcome> {
        if self.popup_open {
            if self.candidates.is_empty() {
                return None;
            }
            self.selected = (self.selected + 1) % self.candidates.len();
            return None;
        }
        let Some(index) = self.history_index else {
            return None;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.buffer = self.history[index + 1].clone();
        } else {
            self.history_index = None;
            self.buffer = self.draft.take().unwrap_or_default();
        }
        self.cursor = self.buffer.len();
        self.sync_popup();
        None
    }

    fn complete_selected(&mut self) {
        if !self.popup_open {
            return;
        }
        if let Some(spec) = self.candidates.get(self.selected).copied() {
            self.apply_completion(spec);
        }
    }

    fn dismiss_popup(&mut self) {
        if !self.popup_open {
            return;
        }
        self.popup_open = false;
        self.selected = 0;
        self.candidates.clear();
        self.dismissed = self.current_token().map(str::to_owned);
    }

    fn submit(&mut self) -> Option<Outcome> {
        if self.popup_open {
            if let Some(spec) = self.candidates.get(self.selected).copied() {
                self.apply_completion(spec);
            }
        }
        Some(Outcome::Submitted)
    }

    fn interrupt_or_clear(&mut self) -> Option<Outcome> {
        if self.buffer.is_empty() {
            return Some(Outcome::Interrupt);
        }
        self.buffer.clear();
        self.cursor = 0;
        self.history_index = None;
        self.draft = None;
        self.sync_popup();
        None
    }

    fn eof_if_empty(&mut self) -> Option<Outcome> {
        if self.buffer.is_empty() {
            Some(Outcome::Eof)
        } else {
            None
        }
    }

    /// 退出历史浏览：当前缓冲成为新草稿。
    fn leave_history(&mut self) {
        self.history_index = None;
        self.draft = None;
    }

    /// 首个空白前 token 的字节末尾；仅当缓冲以 `/` 开头时存在。
    fn first_token_end(&self) -> Option<usize> {
        if !self.buffer.starts_with('/') {
            return None;
        }
        Some(
            self.buffer
                .find(char::is_whitespace)
                .unwrap_or(self.buffer.len()),
        )
    }

    fn current_token(&self) -> Option<&str> {
        let end = self.first_token_end()?;
        Some(&self.buffer[..end])
    }

    fn popup_should_open(&self) -> bool {
        let Some(end) = self.first_token_end() else {
            return false;
        };
        if self.cursor > end {
            return false;
        }
        let token = &self.buffer[..end];
        self.dismissed.as_deref() != Some(token)
    }

    /// 每次缓冲/光标变化后同步弹窗显隐与候选。
    fn sync_popup(&mut self) {
        if self.current_token().map(str::to_owned).as_deref() != self.dismissed.as_deref() {
            self.dismissed = None;
        }
        if self.popup_should_open() {
            let candidates = filter_slash_commands(self.current_token().unwrap_or_default());
            if candidates != self.candidates {
                self.selected = 0;
            }
            if self.selected >= candidates.len() {
                self.selected = 0;
            }
            self.candidates = candidates;
            self.popup_open = !self.candidates.is_empty();
        } else {
            self.popup_open = false;
            self.selected = 0;
            self.candidates.clear();
        }
    }

    /// 应用补全：规范名替换首 token，保留已输参数文本；无参数时补尾随空格。
    fn apply_completion(&mut self, spec: &'static SlashSpec) {
        let Some(end) = self.first_token_end() else {
            return;
        };
        let rest = self.buffer[end..].to_string();
        self.buffer = if rest.is_empty() {
            format!("/{} ", spec.name)
        } else {
            format!("/{}{}", spec.name, rest)
        };
        self.cursor = self.buffer.len();
        self.dismissed = None;
        self.sync_popup();
    }
}

/// 读取一行输入；raw mode 仅在等待输入期间启用。
/// 返回 [`LineResult::Interrupt`]（空行 Ctrl+C）或 [`LineResult::Eof`] 时由调用方决定退出。
pub fn read_line(prompt: &str, history: &mut Vec<String>) -> io::Result<LineResult> {
    enable_raw_mode()?;
    let result = read_line_raw(prompt, history);
    disable_raw_mode()?;
    result
}

pub enum LineResult {
    Line(String),
    Interrupt,
    Eof,
}

/// 把 crossterm 按键事件翻译为编辑器动作；非按键事件返回 `None`。
fn map_event(event: Event) -> Option<KeyAction> {
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        ..
    }) = event
    else {
        return None;
    };
    Some(match (modifiers, code) {
        (m, KeyCode::Char('c')) if m.contains(KeyModifiers::CONTROL) => KeyAction::CtrlC,
        (m, KeyCode::Char('d')) if m.contains(KeyModifiers::CONTROL) => KeyAction::CtrlD,
        (_, KeyCode::Char(c)) => KeyAction::Char(c),
        (_, KeyCode::Backspace) => KeyAction::Backspace,
        (_, KeyCode::Delete) => KeyAction::Delete,
        (_, KeyCode::Left) => KeyAction::Left,
        (_, KeyCode::Right) => KeyAction::Right,
        (_, KeyCode::Home) => KeyAction::Home,
        (_, KeyCode::End) => KeyAction::End,
        (_, KeyCode::Up) => KeyAction::Up,
        (_, KeyCode::Down) => KeyAction::Down,
        (_, KeyCode::Tab) => KeyAction::Tab,
        (_, KeyCode::Esc) => KeyAction::Esc,
        (_, KeyCode::Enter) => KeyAction::Enter,
        _ => KeyAction::Other,
    })
}

fn read_line_raw(prompt: &str, history: &mut Vec<String>) -> io::Result<LineResult> {
    let mut out = io::stdout();
    let mut editor = Editor::new(std::mem::take(history));
    // 输入行固定在提示符出现时的屏幕行；之后全部用绝对行号重绘，
    // 避免相对位移（MoveUp/MoveDown）在 ConPTY 下与滚动/换行叠加产生错位。
    let (_start_x, row) = position()?;
    let screen_rows = size()?.1;
    let max_rows = ((screen_rows.saturating_sub(row + 1)) as usize).min(MAX_POPUP_ROWS);
    let mut drawn_rows = 0usize;
    execute!(out, MoveTo(0, row), Clear(ClearType::UntilNewLine))?;
    let _ = write!(out, "{prompt}");
    out.flush()?;
    loop {
        let Some(action) = map_event(event::read()?) else {
            continue;
        };
        match editor.handle(action) {
            Some(outcome) => {
                clear_popup_area(&mut out, row, drawn_rows)?;
                execute!(out, MoveTo(0, row), Clear(ClearType::UntilNewLine))?;
                writeln!(out)?;
                let buffer = editor.buffer().to_string();
                if outcome == Outcome::Submitted && !buffer.trim().is_empty() {
                    history.push(buffer.clone());
                }
                return Ok(match outcome {
                    Outcome::Submitted => LineResult::Line(buffer),
                    Outcome::Interrupt => LineResult::Interrupt,
                    Outcome::Eof => LineResult::Eof,
                });
            }
            None => {
                drawn_rows = redraw(&mut out, prompt, row, max_rows, &editor, drawn_rows)?;
            }
        }
    }
}

/// 重绘输入行与弹窗区域（绝对行号），返回本次弹窗占用的行数。
fn redraw(
    out: &mut impl Write,
    prompt: &str,
    row: u16,
    max_rows: usize,
    editor: &Editor,
    previous_rows: usize,
) -> io::Result<usize> {
    let prompt_width = display_width(prompt);
    let candidates = editor.popup_candidates();
    let rows = if editor.popup_open() {
        candidates.len().min(max_rows)
    } else {
        0
    };
    queue!(out, MoveTo(0, row), Clear(ClearType::UntilNewLine))?;
    let _ = write!(out, "{prompt}{}", editor.buffer());
    // 清上一帧残留的弹窗行（新帧行数变少时）
    for index in rows..previous_rows {
        queue!(
            out,
            MoveTo(0, row + 1 + index as u16),
            Clear(ClearType::UntilNewLine)
        )?;
    }
    if rows > 0 {
        let selected = editor.popup_selected();
        let start = if selected < rows {
            0
        } else {
            selected + 1 - rows
        };
        for (index, spec) in candidates.iter().skip(start).take(rows).enumerate() {
            let mark = if start + index == selected {
                SELECTED_MARK
            } else {
                ITEM_MARK
            };
            queue!(
                out,
                MoveTo(0, row + 1 + index as u16),
                Clear(ClearType::UntilNewLine)
            )?;
            let _ = write!(out, "{mark}{}  {}", spec.display(), spec.description);
        }
    }
    queue!(
        out,
        MoveTo(
            u16::try_from(prompt_width + editor.cursor_width()).unwrap_or(u16::MAX),
            row
        )
    )?;
    out.flush()?;
    Ok(rows)
}

/// 清除弹窗残留行（提交/退出路径）。
fn clear_popup_area(out: &mut impl Write, row: u16, rows: usize) -> io::Result<()> {
    for index in 0..rows {
        queue!(
            out,
            MoveTo(0, row + 1 + index as u16),
            Clear(ClearType::UntilNewLine)
        )?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash::SLASH_COMMANDS;

    fn editor() -> Editor {
        Editor::new(Vec::new())
    }

    fn type_str(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            editor.handle(KeyAction::Char(c));
        }
    }

    #[test]
    fn typing_chars_inserts_at_cursor() {
        let mut e = editor();
        assert_eq!(e.handle(KeyAction::Char('a')), None);
        e.handle(KeyAction::Char('b'));
        assert_eq!(e.buffer(), "ab");
        e.handle(KeyAction::Left);
        e.handle(KeyAction::Char('X'));
        assert_eq!(e.buffer(), "aXb");
        assert_eq!(e.cursor_bytes(), "aX".len());
    }

    #[test]
    fn backspace_deletes_before_cursor_and_delete_after() {
        let mut e = editor();
        type_str(&mut e, "ab");
        e.handle(KeyAction::Left);
        e.handle(KeyAction::Backspace);
        assert_eq!(e.buffer(), "b");
        assert_eq!(e.cursor_bytes(), 0);
        e.handle(KeyAction::Delete);
        assert_eq!(e.buffer(), "");
    }

    #[test]
    fn home_end_move_cursor_to_boundaries() {
        let mut e = editor();
        type_str(&mut e, "abc");
        e.handle(KeyAction::Home);
        assert_eq!(e.cursor_bytes(), 0);
        e.handle(KeyAction::End);
        assert_eq!(e.cursor_bytes(), 3);
    }

    #[test]
    fn cursor_movement_respects_char_boundaries() {
        let mut e = editor();
        type_str(&mut e, "你好");
        e.handle(KeyAction::Left);
        assert_eq!(e.cursor_bytes(), "你".len());
        e.handle(KeyAction::Left);
        e.handle(KeyAction::Left);
        assert_eq!(e.cursor_bytes(), 0);
        assert_eq!(e.cursor_width(), 0);
    }

    #[test]
    fn wide_chars_measured_for_display() {
        assert_eq!(display_width("你> "), 4);
        assert_eq!(display_width("ab"), 2);
    }

    #[test]
    fn popup_opens_on_slash_and_filters() {
        let mut e = editor();
        e.handle(KeyAction::Char('/'));
        assert!(e.popup_open());
        assert_eq!(e.popup_candidates().len(), SLASH_COMMANDS.len());
        type_str(&mut e, "mo");
        assert_eq!(e.popup_candidates().len(), 1);
        assert_eq!(e.popup_candidates()[0].name, "model");
    }

    #[test]
    fn tab_completes_command_and_closes_popup() {
        let mut e = editor();
        type_str(&mut e, "/mo");
        e.handle(KeyAction::Tab);
        assert_eq!(e.buffer(), "/model ");
        assert!(!e.popup_open());
        assert_eq!(e.cursor_bytes(), "/model ".len());
    }

    #[test]
    fn tab_preserves_typed_args() {
        let mut e = editor();
        type_str(&mut e, "/mo glm");
        // 光标在参数区：弹窗已关，Tab 无操作
        e.handle(KeyAction::Tab);
        assert_eq!(e.buffer(), "/mo glm");
        // 光标移回首 token 内：弹窗重开，Tab 补全命令名并保留参数
        e.handle(KeyAction::Home);
        for _ in 0..3 {
            e.handle(KeyAction::Right);
        }
        assert!(e.popup_open());
        e.handle(KeyAction::Tab);
        assert_eq!(e.buffer(), "/model glm");
    }

    #[test]
    fn enter_with_popup_applies_completion_and_submits() {
        let mut e = editor();
        type_str(&mut e, "/he");
        assert_eq!(e.handle(KeyAction::Enter), Some(Outcome::Submitted));
        assert_eq!(e.buffer(), "/help ");
    }

    #[test]
    fn enter_without_popup_submits_buffer() {
        let mut e = editor();
        type_str(&mut e, "你好");
        assert_eq!(e.handle(KeyAction::Enter), Some(Outcome::Submitted));
        assert_eq!(e.buffer(), "你好");
    }

    #[test]
    fn esc_dismisses_popup_until_token_changes() {
        let mut e = editor();
        e.handle(KeyAction::Char('/'));
        e.handle(KeyAction::Esc);
        assert!(!e.popup_open());
        // token 变化后按新 token 重新过滤弹出
        e.handle(KeyAction::Char('h'));
        assert!(e.popup_open());
        // 再次 Esc 后，仅移动光标（token 不变）不重弹
        e.handle(KeyAction::Esc);
        assert!(!e.popup_open());
        e.handle(KeyAction::Left);
        e.handle(KeyAction::Right);
        assert!(!e.popup_open());
    }

    #[test]
    fn popup_closes_when_cursor_leaves_first_token() {
        let mut e = editor();
        type_str(&mut e, "/mo");
        assert!(e.popup_open());
        e.handle(KeyAction::Char(' '));
        assert!(!e.popup_open());
    }

    #[test]
    fn popup_closes_when_input_no_longer_slash() {
        let mut e = editor();
        e.handle(KeyAction::Char('/'));
        assert!(e.popup_open());
        e.handle(KeyAction::Backspace);
        e.handle(KeyAction::Char('你'));
        assert!(!e.popup_open());
    }

    #[test]
    fn arrows_navigate_popup_with_wrap() {
        let mut e = editor();
        e.handle(KeyAction::Char('/'));
        let total = e.popup_candidates().len();
        e.handle(KeyAction::Down);
        assert_eq!(e.popup_selected(), 1);
        e.handle(KeyAction::Up);
        e.handle(KeyAction::Up);
        assert_eq!(e.popup_selected(), total - 1);
    }

    #[test]
    fn up_down_browse_history_when_popup_closed() {
        let mut e = Editor::new(vec!["first".into(), "second".into()]);
        e.handle(KeyAction::Up);
        assert_eq!(e.buffer(), "second");
        e.handle(KeyAction::Up);
        assert_eq!(e.buffer(), "first");
        e.handle(KeyAction::Down);
        assert_eq!(e.buffer(), "second");
        e.handle(KeyAction::Down);
        assert_eq!(e.buffer(), "");
    }

    #[test]
    fn up_down_keeps_draft() {
        let mut e = Editor::new(vec!["old".into()]);
        type_str(&mut e, "草稿");
        e.handle(KeyAction::Up);
        assert_eq!(e.buffer(), "old");
        e.handle(KeyAction::Down);
        assert_eq!(e.buffer(), "草稿");
    }

    #[test]
    fn ctrl_c_clears_nonempty_line_then_interrupts_on_empty() {
        let mut e = editor();
        e.handle(KeyAction::Char('x'));
        assert_eq!(e.handle(KeyAction::CtrlC), None);
        assert_eq!(e.buffer(), "");
        assert_eq!(e.handle(KeyAction::CtrlC), Some(Outcome::Interrupt));
    }

    #[test]
    fn ctrl_d_yields_eof_only_on_empty_buffer() {
        let mut e = editor();
        e.handle(KeyAction::Char('x'));
        assert_eq!(e.handle(KeyAction::CtrlD), None);
        e.handle(KeyAction::Backspace);
        assert_eq!(e.handle(KeyAction::CtrlD), Some(Outcome::Eof));
    }
}
