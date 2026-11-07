//! 工作副本改动的 unified diff 渲染与 apply 选择（docs/specs/cli.md diff 契约）。
//! 渲染语义与桌面端共享自 harness-platform；此处仅保留 CLI 专属的着色开关与交互选择。
use clap::ValueEnum;
use harness_contracts::FileChange;
pub use harness_platform::diffview::{
    Binary, MAX_DIFF_BYTES, MAX_DIFF_LINES, fallback_note, render, sides,
};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// auto 依 TTY 决定着色。
    pub fn effective(self, is_tty: bool) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => is_tty,
        }
    }
}

/// 变更清单行：path、kind 与冲突标注。
pub fn summary_line(change: &FileChange) -> String {
    let mut line = format!("{} [{}]", change.path, change.kind);
    if change.conflict {
        line.push_str(" CONFLICT（暂不可 apply，需先处理原项目侧改动）");
    }
    line
}

/// 交互式确认回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    All,
    Quit,
}

/// apply 选择：返回 (选中项, 排除的冲突路径, 是否中止)。冲突文件始终自动排除。
pub fn select(
    changes: &[FileChange],
    all: bool,
    mut ask: impl FnMut(&FileChange) -> Answer,
) -> (Vec<FileChange>, Vec<String>, bool) {
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let mut rest = all;
    for change in changes {
        if change.conflict {
            skipped.push(change.path.clone());
            continue;
        }
        if !rest {
            match ask(change) {
                Answer::Yes => {}
                Answer::No => continue,
                Answer::All => rest = true,
                Answer::Quit => return (selected, skipped, true),
            }
        }
        selected.push(change.clone());
    }
    (selected, skipped, false)
}

/// 向 writer 输出 diff 清单。
pub fn write_listing(
    out: &mut dyn Write,
    changes: &[FileChange],
    color: bool,
) -> std::io::Result<()> {
    for change in changes {
        writeln!(out, "{}", summary_line(change))?;
        if let Some(diff) = render(change, color) {
            write!(out, "{diff}")?;
        } else {
            write!(out, "{}", fallback_note(change))?;
        }
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(kind: &str, before: Option<&str>, after: Option<&str>, conflict: bool) -> FileChange {
        serde_json::from_value(json!({
            "path":"a.txt","kind":kind,
            "beforeHash": before.is_some().then_some("bh"),
            "afterHash": after.is_some().then_some("ah"),
            "sourceHash": serde_json::Value::Null,
            "conflict": conflict,
            "beforeText": before,
            "afterText": after,
        }))
        .unwrap()
    }

    #[test]
    fn rendered_diff_is_shared_and_uncolored_by_default() {
        let rendered = render(&change("added", None, Some("new line"), false), false).unwrap();
        assert!(rendered.contains("+new line"));
        assert!(!rendered.contains('\x1b'));
    }

    #[test]
    fn conflict_is_marked_and_excluded_from_selection() {
        let line = summary_line(&change("modified", Some("a"), Some("b"), true));
        assert!(line.contains("CONFLICT"));
        let changes = vec![
            change("modified", Some("a"), Some("b"), true),
            change("modified", Some("a"), Some("c"), false),
        ];
        let (selected, skipped, aborted) = select(&changes, true, |_| Answer::Yes);
        assert!(!aborted);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].path, "a.txt");
        assert_eq!(skipped.len(), 1);
    }

    #[test]
    fn interactive_selection_supports_yes_no_all_and_quit() {
        let changes = vec![
            change("modified", Some("a"), Some("1"), false),
            change("modified", Some("a"), Some("2"), false),
            change("modified", Some("a"), Some("3"), false),
        ];
        // y, n, y → 选中第 1、3 项
        let mut answers = vec![Answer::Yes, Answer::No, Answer::Yes];
        let (selected, _, _) = select(&changes, false, |_| answers.remove(0));
        assert_eq!(selected.len(), 2);
        // 首个即 All → 全部剩余
        let (selected, _, _) = select(&changes, false, |_| Answer::All);
        assert_eq!(selected.len(), 3);
        // Quit → 中止，已确认项不生效
        let (selected, _, aborted) = select(&changes, false, |_| Answer::Quit);
        assert!(selected.is_empty());
        assert!(aborted);
    }

    #[test]
    fn color_choice_resolves_by_tty() {
        assert!(!ColorChoice::Never.effective(true));
        assert!(ColorChoice::Auto.effective(true));
        assert!(!ColorChoice::Auto.effective(false));
    }

    #[test]
    fn listing_writes_summary_and_fallback_note_for_binary() {
        let mut out = Vec::new();
        let changes = vec![change("modified", None, Some("x"), false)];
        write_listing(&mut out, &changes, false).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("a.txt [modified]"));
        assert!(text.contains("不渲染正文"));
    }
}
