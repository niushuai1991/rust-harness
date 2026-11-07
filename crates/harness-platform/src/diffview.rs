//! 工作副本改动的 unified diff 渲染（docs/specs/cli-spec.md diff 契约）。
//! CLI 与桌面端共享同一渲染语义；CLI 专属的着色与交互选择见 harness-cli。
use harness_contracts::FileChange;
use similar::TextDiff;

/// 渲染前置字节阈值：任一侧超过即不生成 unified diff（平台基线读取无上限）。
pub const MAX_DIFF_BYTES: usize = 1 << 20;
/// 显示层截断：单文件 diff 输出最大行数。
pub const MAX_DIFF_LINES: usize = 2000;

/// 文本缺失解释失败：modified 且单侧缺失视为二进制。
#[derive(Debug, PartialEq, Eq)]
pub struct Binary;

/// 按 kind 解释文本缺失：added→before 视为空；deleted→after 视为空；modified 单侧缺失为二进制。
pub fn sides(change: &FileChange) -> Result<(String, String), Binary> {
    match change.kind.as_str() {
        "added" => {
            let after = change.after_text.as_deref().ok_or(Binary)?;
            Ok((
                change.before_text.as_deref().unwrap_or("").to_owned(),
                after.to_owned(),
            ))
        }
        "deleted" => {
            let before = change.before_text.as_deref().ok_or(Binary)?;
            Ok((
                before.to_owned(),
                change.after_text.as_deref().unwrap_or("").to_owned(),
            ))
        }
        _ => {
            let before = change.before_text.as_deref().ok_or(Binary)?;
            let after = change.after_text.as_deref().ok_or(Binary)?;
            Ok((before.to_owned(), after.to_owned()))
        }
    }
}

/// 渲染单文件 unified diff；文本缺失或超过字节阈值返回 None（调用方显示哈希提示）。
pub fn render(change: &FileChange, color: bool) -> Option<String> {
    let (before, after) = sides(change).ok()?;
    if before.len() > MAX_DIFF_BYTES || after.len() > MAX_DIFF_BYTES {
        return None;
    }
    let text_diff = TextDiff::from_lines(&before, &after);
    let mut diff = text_diff.unified_diff();
    diff.context_radius(3);
    diff.header(&change.path, &change.path);
    let text = diff.to_string();
    let truncated_at_end = text.lines().count() > MAX_DIFF_LINES;
    let truncated: Vec<&str> = text.lines().take(MAX_DIFF_LINES).collect();
    let mut out = truncated.join("\n");
    out.push('\n');
    if truncated_at_end {
        out.push_str(&format!("…（diff 超过 {MAX_DIFF_LINES} 行，已截断显示）\n"));
    }
    if color {
        out = colorize(&out);
    }
    Some(out)
}

fn colorize(diff: &str) -> String {
    diff.lines()
        .map(|line| {
            let colored = if line.starts_with('+') && !line.starts_with("+++") {
                format!("\x1b[32m{line}\x1b[0m")
            } else if line.starts_with('-') && !line.starts_with("---") {
                format!("\x1b[31m{line}\x1b[0m")
            } else if line.starts_with("@@") {
                format!("\x1b[36m{line}\x1b[0m")
            } else {
                line.to_owned()
            };
            colored + "\n"
        })
        .collect()
}

/// 文本缺失或超阈值时的提示（显示哈希）。
pub fn fallback_note(change: &FileChange) -> String {
    format!(
        "{} [{}] 不渲染正文：{}\n  before={} after={}\n",
        change.path,
        change.kind,
        if sides(change).is_err() {
            "二进制或无法解码"
        } else {
            "文本超过 1MiB"
        },
        change.before_hash.as_deref().unwrap_or("-"),
        change.after_hash.as_deref().unwrap_or("-"),
    )
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
    fn added_treats_missing_before_as_empty() {
        let (before, after) = sides(&change("added", None, Some("new"), false)).unwrap();
        assert_eq!(before, "");
        assert_eq!(after, "new");
        let rendered = render(&change("added", None, Some("new line"), false), false).unwrap();
        assert!(rendered.contains("+new line"));
        assert!(rendered.contains("--- a.txt"));
    }

    #[test]
    fn deleted_treats_missing_after_as_empty() {
        let (before, after) = sides(&change("deleted", Some("old"), None, false)).unwrap();
        assert_eq!(before, "old");
        assert_eq!(after, "");
        let rendered = render(&change("deleted", Some("old line"), None, false), false).unwrap();
        assert!(rendered.contains("-old line"));
    }

    #[test]
    fn modified_with_missing_side_is_binary() {
        assert_eq!(
            sides(&change("modified", None, Some("x"), false)),
            Err(Binary)
        );
        assert_eq!(
            sides(&change("modified", Some("x"), None, false)),
            Err(Binary)
        );
        assert!(render(&change("modified", None, Some("x"), false), false).is_none());
    }

    #[test]
    fn oversize_text_is_not_rendered() {
        let huge = "x".repeat(MAX_DIFF_BYTES + 1);
        assert!(render(&change("modified", Some("a"), Some(&huge), false), false).is_none());
        let note = fallback_note(&change("modified", Some("a"), Some(&huge), false));
        assert!(note.contains("1MiB"));
    }

    #[test]
    fn long_diff_is_truncated_at_display_limit() {
        let after: String = (0..(MAX_DIFF_LINES + 200))
            .map(|i| format!("line{i}\n"))
            .collect();
        let rendered = render(&change("added", None, Some(&after), false), false).unwrap();
        assert!(rendered.lines().count() <= MAX_DIFF_LINES + 2);
        assert!(rendered.contains("已截断显示"));
    }

    #[test]
    fn color_codes_applied_only_when_enabled() {
        let plain = render(&change("added", None, Some("x"), false), false).unwrap();
        let colored = render(&change("added", None, Some("x"), false), true).unwrap();
        assert!(!plain.contains('\x1b'));
        assert!(colored.contains("\x1b[32m+x\x1b[0m"));
    }
}
