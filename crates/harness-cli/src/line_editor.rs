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

/// 模型候选行；由 chat 每轮刷新的目录快照投影而来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    /// 完整模型 id（如 `builtin:bigmodel:glm-5.3`）。
    pub id: String,
    /// 展示名（缺省回落模型标识）。
    pub label: String,
    /// 供应商展示名（弹窗右对齐列）。
    pub provider: String,
    /// 是否为会话当前模型。
    pub current: bool,
}

/// 档位候选行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortRow {
    pub level: String,
    pub current: bool,
}

/// 弹窗候选行：斜杠命令 / 模型 / 档位 / 技能。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    Slash(&'static SlashSpec),
    Model(ModelRow),
    Effort(EffortRow),
    Skill(SkillRow),
}

impl Candidate {
    /// 左列主文本（选中标记之后）。
    fn primary(&self) -> String {
        match self {
            Candidate::Slash(spec) => spec.display(),
            Candidate::Model(row) => row.label.clone(),
            Candidate::Effort(row) => row.level.clone(),
            Candidate::Skill(row) => format!("${}", row.name),
        }
    }

    /// 主文本之后的次要说明。
    fn meta(&self) -> String {
        match self {
            Candidate::Slash(spec) => spec.description.to_owned(),
            Candidate::Model(row) => {
                if row.current {
                    "当前".to_owned()
                } else {
                    String::new()
                }
            }
            Candidate::Effort(row) => {
                if row.current {
                    "当前".to_owned()
                } else {
                    String::new()
                }
            }
            Candidate::Skill(row) => row.description.clone(),
        }
    }

    /// 右对齐列（仅模型行）。
    fn provider(&self) -> Option<&str> {
        match self {
            Candidate::Model(row) => Some(row.provider.as_str()),
            _ => None,
        }
    }
}

/// 技能候选行（`$` 补全用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
}

/// 内联面板上下文：模型目录、当前模型档位与技能名单；chat 每轮传入，不做缓存。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerContext {
    pub models: Vec<ModelRow>,
    pub efforts: Vec<String>,
    pub current_effort: Option<String>,
    pub skills: Vec<SkillRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopupMode {
    Slash,
    Models,
    Efforts,
    Skills,
}

/// 大小写不敏感子串过滤模型行：匹配 id、名称与供应商；空查询返回全量。
pub fn filter_model_rows(rows: &[ModelRow], query: &str) -> Vec<Candidate> {
    let query = query.trim().to_ascii_lowercase();
    rows.iter()
        .filter(|row| {
            query.is_empty()
                || row.id.to_ascii_lowercase().contains(&query)
                || row.label.to_ascii_lowercase().contains(&query)
                || row.provider.to_ascii_lowercase().contains(&query)
        })
        .map(|row| Candidate::Model(row.clone()))
        .collect()
}

/// 大小写不敏感子串过滤技能行：匹配 name 与描述；空查询返回全量，保持目录排序。
pub fn filter_skill_rows(rows: &[SkillRow], query: &str) -> Vec<Candidate> {
    let query = query.trim().to_ascii_lowercase();
    rows.iter()
        .filter(|row| {
            query.is_empty()
                || row.name.to_ascii_lowercase().contains(&query)
                || row.description.to_ascii_lowercase().contains(&query)
        })
        .map(|row| Candidate::Skill(row.clone()))
        .collect()
}

/// 大小写不敏感子串过滤档位行；空查询返回全量，顺序保持档位声明序。
pub fn filter_effort_rows(levels: &[String], current: Option<&str>, query: &str) -> Vec<Candidate> {
    let query = query.trim().to_ascii_lowercase();
    levels
        .iter()
        .filter(|level| query.is_empty() || level.to_ascii_lowercase().contains(&query))
        .map(|level| {
            Candidate::Effort(EffortRow {
                level: level.clone(),
                current: Some(level.as_str()) == current,
            })
        })
        .collect()
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
    candidates: Vec<Candidate>,
    selected: usize,
    popup_open: bool,
    /// Esc 关闭弹窗时的首 token；同一 token 内不再自动弹出。
    dismissed: Option<String>,
    /// 模型/档位面板的数据快照（chat 每轮传入）。
    picker: PickerContext,
}

impl Editor {
    pub fn new(history: Vec<String>, picker: PickerContext) -> Self {
        Self {
            history,
            picker,
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

    pub fn popup_candidates(&self) -> &[Candidate] {
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
        match self.candidates.get(self.selected).cloned() {
            Some(Candidate::Slash(spec)) => self.apply_completion(spec),
            // Tab 补全参数并收起面板；Enter 的完整改写在 [`Editor::submit`] 中处理。
            Some(Candidate::Model(row)) => {
                self.buffer = format!("/model {}", row.id);
                self.cursor = self.buffer.len();
                self.dismissed = self.current_token().map(str::to_owned);
                self.sync_popup();
            }
            Some(Candidate::Effort(row)) => {
                self.buffer = format!("/effort {}", row.level);
                self.cursor = self.buffer.len();
                self.dismissed = self.current_token().map(str::to_owned);
                self.sync_popup();
            }
            Some(Candidate::Skill(row)) => self.apply_skill_completion(&row.name),
            None => {}
        }
    }

    fn dismiss_popup(&mut self) {
        if !self.popup_open {
            return;
        }
        self.popup_open = false;
        self.selected = 0;
        self.candidates.clear();
        // 斜杠上下文记录首 token；`$` 上下文记录光标处 token，同一 token 内不再自动弹出。
        self.dismissed = if self.buffer.starts_with('/') {
            self.current_token().map(str::to_owned)
        } else {
            self.token_at_cursor()
                .map(|(start, end)| self.buffer[start..end].to_owned())
        };
    }

    fn submit(&mut self) -> Option<Outcome> {
        if self.popup_open {
            match self.candidates.get(self.selected).cloned() {
                // Enter 选中模型/档位时把缓冲改写为完整命令再提交，避免手打长 id。
                Some(Candidate::Model(row)) => {
                    self.buffer = format!("/model {}", row.id);
                    self.cursor = self.buffer.len();
                }
                Some(Candidate::Effort(row)) => {
                    self.buffer = format!("/effort {}", row.level);
                    self.cursor = self.buffer.len();
                }
                Some(Candidate::Slash(spec)) => self.apply_completion(spec),
                // 技能补全只改写 `$token` 为完整引用，消息还要继续写，不提交。
                Some(Candidate::Skill(row)) => {
                    self.apply_skill_completion(&row.name);
                    return None;
                }
                None => {}
            }
        }
        Some(Outcome::Submitted)
    }

    /// 把光标处 `$token` 改写为 `$name `（token 后已有空格则不重复补），用于技能补全。
    fn apply_skill_completion(&mut self, name: &str) {
        let Some((start, end)) = self.token_at_cursor() else {
            return;
        };
        let completed = if self.buffer[end..].starts_with(' ') {
            format!("${name}")
        } else {
            format!("${name} ")
        };
        self.buffer.replace_range(start..end, &completed);
        self.cursor = start + completed.len();
        // 已选定该技能：同一 `$name` token 内不再弹窗，继续输入后按新 token 重新过滤。
        self.dismissed = Some(format!("${name}"));
        self.sync_popup();
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

    /// 光标所在 token 的字节区间；光标越过 token 末尾后返回 None。
    /// `$` 补全可出现在消息任意位置，不受首 token `/` 约束。
    fn token_at_cursor(&self) -> Option<(usize, usize)> {
        let mut start = self.buffer[..self.cursor]
            .rfind(char::is_whitespace)
            .map_or(0, |i| i + 1);
        while start < self.cursor && !self.buffer.is_char_boundary(start) {
            start += 1;
        }
        let end = self.buffer[start..]
            .find(char::is_whitespace)
            .map_or(self.buffer.len(), |i| start + i);
        if self.cursor > end {
            return None;
        }
        Some((start, end))
    }

    /// 当前弹窗模式：自由文本中光标处 `$token` 触发技能补全；
    /// 斜杠上下文（缓冲以 `/` 开头）保持原有命令/参数面板逻辑，不受 `$` 干扰。
    fn popup_mode(&self) -> Option<PopupMode> {
        if self.buffer.starts_with('/') {
            let end = self.first_token_end()?;
            let token = &self.buffer[..end];
            if self.dismissed.as_deref() == Some(token) {
                return None;
            }
            if token == "/model" && !self.picker.models.is_empty() {
                return Some(PopupMode::Models);
            }
            if token == "/effort" && !self.picker.efforts.is_empty() {
                return Some(PopupMode::Efforts);
            }
            if self.cursor <= end {
                return Some(PopupMode::Slash);
            }
            return None;
        }
        let (start, end) = self.token_at_cursor()?;
        let token = &self.buffer[start..end];
        if !token.starts_with('$') || self.dismissed.as_deref() == Some(token) {
            return None;
        }
        if !self.picker.skills.is_empty() {
            return Some(PopupMode::Skills);
        }
        None
    }

    /// 光标处 `$token` 去掉前缀后的查询文本。
    fn skill_query(&self) -> String {
        match self.token_at_cursor() {
            Some((start, end)) => self.buffer[start..end]
                .strip_prefix('$')
                .unwrap_or_default()
                .to_owned(),
            None => String::new(),
        }
    }

    /// `/model`、`/effort` 之后已输入的参数文本。
    fn argument_query(&self) -> String {
        match self.first_token_end() {
            Some(end) => self.buffer[end..].to_owned(),
            None => String::new(),
        }
    }

    /// 每次缓冲/光标变化后同步弹窗显隐与候选。
    fn sync_popup(&mut self) {
        // dismissed 记录的 token 与当前上下文（斜杠首 token / 光标处 token）一致时保持关闭。
        let current = if self.buffer.starts_with('/') {
            self.current_token().map(str::to_owned)
        } else {
            self.token_at_cursor()
                .map(|(start, end)| self.buffer[start..end].to_owned())
        };
        if current.as_deref() != self.dismissed.as_deref() {
            self.dismissed = None;
        }
        let candidates = match self.popup_mode() {
            Some(PopupMode::Slash) => {
                filter_slash_commands(self.current_token().unwrap_or_default())
                    .into_iter()
                    .map(Candidate::Slash)
                    .collect()
            }
            Some(PopupMode::Models) => {
                filter_model_rows(&self.picker.models, &self.argument_query())
            }
            Some(PopupMode::Efforts) => filter_effort_rows(
                &self.picker.efforts,
                self.picker.current_effort.as_deref(),
                &self.argument_query(),
            ),
            Some(PopupMode::Skills) => filter_skill_rows(&self.picker.skills, &self.skill_query()),
            None => Vec::new(),
        };
        if candidates != self.candidates {
            self.selected = 0;
        }
        if self.selected >= candidates.len() {
            self.selected = 0;
        }
        self.candidates = candidates;
        self.popup_open = !self.candidates.is_empty();
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
/// `picker` 提供模型/档位面板的目录快照，由调用方每轮刷新。
/// 返回 [`LineResult::Interrupt`]（空行 Ctrl+C）或 [`LineResult::Eof`] 时由调用方决定退出。
pub fn read_line(
    prompt: &str,
    history: &mut Vec<String>,
    picker: PickerContext,
) -> io::Result<LineResult> {
    enable_raw_mode()?;
    let result = read_line_raw(prompt, history, picker);
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

fn read_line_raw(
    prompt: &str,
    history: &mut Vec<String>,
    picker: PickerContext,
) -> io::Result<LineResult> {
    let mut out = io::stdout();
    let mut editor = Editor::new(std::mem::take(history), picker);
    // 输入行固定在提示符出现时的屏幕行；之后全部用绝对行号重绘，
    // 避免相对位移（MoveUp/MoveDown）在 ConPTY 下与滚动/换行叠加产生错位。
    let (_start_x, row) = position()?;
    let (screen_cols, screen_rows) = size()?;
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
                drawn_rows = redraw(
                    &mut out,
                    prompt,
                    row,
                    screen_cols,
                    max_rows,
                    &editor,
                    drawn_rows,
                )?;
            }
        }
    }
}

/// 重绘输入行与弹窗区域（绝对行号），返回本次弹窗占用的行数。
fn redraw(
    out: &mut impl Write,
    prompt: &str,
    row: u16,
    cols: u16,
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
        for (index, candidate) in candidates.iter().skip(start).take(rows).enumerate() {
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
            let left = format!("{mark}{}", candidate.primary());
            let meta = candidate.meta();
            if meta.is_empty() {
                let _ = write!(out, "{left}");
            } else {
                let _ = write!(out, "{left}  {meta}");
            }
            // 模型行的供应商列右对齐到终端右缘；空间不足两个空格时省略。
            if let Some(provider) = candidate.provider().filter(|name| !name.is_empty()) {
                let used = display_width(&left)
                    + if meta.is_empty() { 0 } else { 2 }
                    + display_width(provider);
                let pad = (cols as usize).saturating_sub(used);
                if pad >= 2 {
                    let _ = write!(out, "{}{provider}", " ".repeat(pad));
                }
            }
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
        Editor::new(Vec::new(), PickerContext::default())
    }

    fn picker() -> PickerContext {
        PickerContext {
            models: vec![
                ModelRow {
                    id: "builtin:bigmodel:glm-5.3".into(),
                    label: "GLM-5.3".into(),
                    provider: "智谱 Coding Plan".into(),
                    current: true,
                },
                ModelRow {
                    id: "builtin:deepseek:deepseek-flash".into(),
                    label: "DeepSeek Flash".into(),
                    provider: "DeepSeek".into(),
                    current: false,
                },
            ],
            efforts: vec!["low".into(), "high".into(), "max".into()],
            current_effort: Some("high".into()),
            skills: Vec::new(),
        }
    }

    fn slash_names(e: &Editor) -> Vec<&'static str> {
        e.popup_candidates()
            .iter()
            .filter_map(|candidate| match candidate {
                Candidate::Slash(spec) => Some(spec.name),
                _ => None,
            })
            .collect()
    }

    fn model_rows(e: &Editor) -> Vec<ModelRow> {
        e.popup_candidates()
            .iter()
            .filter_map(|candidate| match candidate {
                Candidate::Model(row) => Some(row.clone()),
                _ => None,
            })
            .collect()
    }

    fn effort_levels(e: &Editor) -> Vec<String> {
        e.popup_candidates()
            .iter()
            .filter_map(|candidate| match candidate {
                Candidate::Effort(row) => Some(row.level.clone()),
                _ => None,
            })
            .collect()
    }

    fn type_str(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            editor.handle(KeyAction::Char(c));
        }
    }

    fn skill_picker() -> PickerContext {
        PickerContext {
            skills: vec![
                SkillRow {
                    name: "review".into(),
                    description: "代码审查".into(),
                },
                SkillRow {
                    name: "deploy".into(),
                    description: "部署".into(),
                },
            ],
            ..PickerContext::default()
        }
    }

    fn skill_candidates(e: &Editor) -> Vec<String> {
        e.popup_candidates()
            .iter()
            .filter_map(|candidate| match candidate {
                Candidate::Skill(row) => Some(row.name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn dollar_token_opens_skill_popup_anywhere_in_message() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "请先 $rev");
        assert!(e.popup_open(), "光标处 $token 触发技能补全");
        assert_eq!(skill_candidates(&e), vec!["review".to_string()]);
    }

    #[test]
    fn dollar_completion_replaces_token_and_keeps_tail() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "$rev");
        e.handle(KeyAction::Tab);
        assert_eq!(e.buffer(), "$review ");
        assert_eq!(e.cursor_bytes(), "$review ".len());
        // 尾部文本保留：光标移回 token 内再补全
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "$rev x");
        for _ in 0..2 {
            e.handle(KeyAction::Left);
        }
        e.handle(KeyAction::Enter);
        assert_eq!(e.buffer(), "$review x", "Enter 补全 $token 不提交整行");
        assert!(!e.popup_open());
    }

    #[test]
    fn slash_context_keeps_command_popup_over_dollar() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "/model $rev");
        assert!(
            skill_candidates(&e).is_empty(),
            "斜杠命令上下文中 $token 不触发技能弹窗"
        );
    }

    #[test]
    fn plain_text_without_dollar_stays_closed() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "普通消息 review");
        assert!(!e.popup_open());
    }

    #[test]
    fn esc_dismisses_dollar_popup_until_token_changes() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "$rev");
        assert!(e.popup_open());
        e.handle(KeyAction::Esc);
        assert!(!e.popup_open());
        // 仅移动光标（token 不变）不重弹，语义与斜杠弹窗一致
        e.handle(KeyAction::Left);
        e.handle(KeyAction::Right);
        assert!(!e.popup_open());
        type_str(&mut e, " $dep");
        assert!(e.popup_open(), "换新 token 重新弹出");
        assert_eq!(skill_candidates(&e), vec!["deploy".to_string()]);
    }

    #[test]
    fn skill_popup_filters_by_query_case_insensitive() {
        let mut e = Editor::new(Vec::new(), skill_picker());
        type_str(&mut e, "$DEP");
        assert_eq!(skill_candidates(&e), vec!["deploy".to_string()]);
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
        assert_eq!(slash_names(&e), vec!["model"]);
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
        let mut e = Editor::new(
            vec!["first".into(), "second".into()],
            PickerContext::default(),
        );
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
        let mut e = Editor::new(vec!["old".into()], PickerContext::default());
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

    #[test]
    fn model_panel_opens_on_model_token_and_filters() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/model");
        assert!(e.popup_open());
        assert_eq!(model_rows(&e).len(), 2);
        assert!(model_rows(&e)[0].current);
        // 继续输入即过滤：命中 id 与展示名
        type_str(&mut e, " deep");
        assert_eq!(
            model_rows(&e)
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["builtin:deepseek:deepseek-flash"]
        );
        e.handle(KeyAction::Backspace);
        e.handle(KeyAction::Backspace);
        e.handle(KeyAction::Backspace);
        e.handle(KeyAction::Backspace);
        type_str(&mut e, "glm");
        assert_eq!(
            model_rows(&e)
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            vec!["GLM-5.3"]
        );
    }

    #[test]
    fn model_panel_enter_rewrites_command_and_submits() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/model deep");
        assert_eq!(e.handle(KeyAction::Enter), Some(Outcome::Submitted));
        assert_eq!(e.buffer(), "/model builtin:deepseek:deepseek-flash");
    }

    #[test]
    fn model_panel_tab_completes_and_keeps_panel_dismissed() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/model");
        e.handle(KeyAction::Tab);
        assert_eq!(e.buffer(), "/model builtin:bigmodel:glm-5.3");
        assert!(!e.popup_open());
        // Tab 后继续输入不重弹（同 token 内已主动补全）
        e.handle(KeyAction::Char(' '));
        assert!(!e.popup_open());
    }

    #[test]
    fn model_panel_esc_dismisses_then_backspace_restores_slash_popup() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/model");
        e.handle(KeyAction::Esc);
        assert!(!e.popup_open());
        e.handle(KeyAction::Backspace);
        assert_eq!(e.buffer(), "/mode");
        assert!(e.popup_open());
        assert_eq!(slash_names(&e), vec!["model"]);
    }

    #[test]
    fn effort_panel_lists_levels_and_marks_current() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/effort");
        assert!(e.popup_open());
        assert_eq!(effort_levels(&e), vec!["low", "high", "max"]);
        type_str(&mut e, " hi");
        assert_eq!(effort_levels(&e), vec!["high"]);
    }

    #[test]
    fn effort_panel_enter_rewrites_command_and_submits() {
        let mut e = Editor::new(Vec::new(), picker());
        type_str(&mut e, "/effort");
        e.handle(KeyAction::Down);
        e.handle(KeyAction::Down);
        assert_eq!(e.handle(KeyAction::Enter), Some(Outcome::Submitted));
        assert_eq!(e.buffer(), "/effort max");
    }

    #[test]
    fn model_token_without_catalog_falls_back_to_slash_completion() {
        // 目录快照为空时 /model 仍是普通命令补全，不弹空面板
        let mut e = editor();
        type_str(&mut e, "/model");
        assert!(e.popup_open());
        assert_eq!(slash_names(&e), vec!["model"]);
    }

    #[test]
    fn filter_model_rows_matches_id_label_and_provider() {
        let rows = picker().models;
        assert_eq!(filter_model_rows(&rows, "bigmodel").len(), 1);
        assert_eq!(filter_model_rows(&rows, "智谱").len(), 1);
        assert_eq!(filter_model_rows(&rows, "GLM").len(), 1);
        assert_eq!(filter_model_rows(&rows, "").len(), 2);
        assert!(filter_model_rows(&rows, "zzz").is_empty());
    }

    #[test]
    fn filter_effort_rows_is_case_insensitive_and_marks_current() {
        let levels = vec!["low".to_owned(), "high".to_owned()];
        let rows = filter_effort_rows(&levels, Some("high"), "H");
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            Candidate::Effort(row) => {
                assert_eq!(row.level, "high");
                assert!(row.current);
            }
            _ => panic!("应为档位候选"),
        }
    }
}
