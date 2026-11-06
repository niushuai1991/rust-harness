//! 斜杠命令注册表：元数据、过滤与解析，chat REPL 与补全弹窗共用。

/// 单条斜杠命令的元数据；`SLASH_COMMANDS` 顺序即弹窗展示顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashSpec {
    /// 规范名（不含前导 `/`）。
    pub name: &'static str,
    /// 别名，解析与过滤均支持。
    pub aliases: &'static [&'static str],
    /// 弹窗与帮助中的一行描述。
    pub description: &'static str,
    /// 参数提示；`None` 表示无参数。
    pub arg_hint: Option<&'static str>,
}

impl SlashSpec {
    /// 展示用主名，如 `/model [id]`。
    pub fn display(&self) -> String {
        match self.arg_hint {
            Some(hint) => format!("/{} {hint}", self.name),
            None => format!("/{}", self.name),
        }
    }
}

pub static SLASH_COMMANDS: &[SlashSpec] = &[
    SlashSpec {
        name: "help",
        aliases: &["h"],
        description: "显示可用命令",
        arg_hint: None,
    },
    SlashSpec {
        name: "model",
        aliases: &[],
        description: "查看或切换模型",
        arg_hint: Some("[id]"),
    },
    SlashSpec {
        name: "status",
        aliases: &[],
        description: "显示会话与运行状态",
        arg_hint: None,
    },
    SlashSpec {
        name: "clear",
        aliases: &["new"],
        description: "重开新会话",
        arg_hint: None,
    },
    SlashSpec {
        name: "compact",
        aliases: &[],
        description: "压缩历史为摘要",
        arg_hint: None,
    },
    SlashSpec {
        name: "init",
        aliases: &[],
        description: "在项目根生成 AGENTS.md",
        arg_hint: None,
    },
    SlashSpec {
        name: "plan",
        aliases: &[],
        description: "制定实施计划",
        arg_hint: Some("[任务]"),
    },
    SlashSpec {
        name: "goal",
        aliases: &[],
        description: "设定会话目标",
        arg_hint: Some("[目标]"),
    },
    SlashSpec {
        name: "resume",
        aliases: &[],
        description: "切换到已有会话",
        arg_hint: Some("[id]"),
    },
    SlashSpec {
        name: "sessions",
        aliases: &[],
        description: "列出会话",
        arg_hint: None,
    },
    SlashSpec {
        name: "diff",
        aliases: &[],
        description: "查看副本改动",
        arg_hint: None,
    },
    SlashSpec {
        name: "apply",
        aliases: &[],
        description: "审阅后同步到原项目",
        arg_hint: None,
    },
    SlashSpec {
        name: "approval",
        aliases: &[],
        description: "查看或切换审批策略",
        arg_hint: Some("[auto|ask|never]"),
    },
    SlashSpec {
        name: "quit",
        aliases: &["exit", "q"],
        description: "退出 chat",
        arg_hint: None,
    },
];

/// 按规范名或别名查找命令（不含前导 `/`）。
pub fn find_spec(name: &str) -> Option<&'static SlashSpec> {
    SLASH_COMMANDS
        .iter()
        .find(|spec| spec.name == name || spec.aliases.contains(&name))
}

/// 过滤候选：exact 命中优先于 prefix 命中，各自保持注册顺序；空查询返回全量。
pub fn filter_slash_commands(query: &str) -> Vec<&'static SlashSpec> {
    let query = query.trim_start_matches('/').to_ascii_lowercase();
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for spec in SLASH_COMMANDS {
        let hit_exact = spec.name == query || spec.aliases.contains(&query.as_str());
        let hit_prefix = spec.name.starts_with(&query)
            || spec.aliases.iter().any(|alias| alias.starts_with(&query));
        if hit_exact {
            exact.push(spec);
        } else if hit_prefix {
            prefix.push(spec);
        }
    }
    exact.into_iter().chain(prefix).collect()
}

/// 斜杠命令解析；非斜杠输入为消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand<'a> {
    Quit,
    Help,
    Model(Option<&'a str>),
    Clear,
    Compact,
    Init,
    Plan(Option<&'a str>),
    Goal(Option<&'a str>),
    Status,
    Resume(Option<&'a str>),
    Sessions,
    Diff,
    Apply,
    Approval(Option<&'a str>),
    Unknown(&'a str),
    Message,
}

pub fn parse_command(line: &str) -> ChatCommand<'_> {
    if !line.starts_with('/') {
        return ChatCommand::Message;
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    let rest = parts.next().map(str::trim).filter(|rest| !rest.is_empty());
    let Some(spec) = name.strip_prefix('/').and_then(find_spec) else {
        return ChatCommand::Unknown(name);
    };
    match spec.name {
        "quit" => ChatCommand::Quit,
        "help" => ChatCommand::Help,
        "model" => ChatCommand::Model(rest),
        "clear" => ChatCommand::Clear,
        "compact" => ChatCommand::Compact,
        "init" => ChatCommand::Init,
        "plan" => ChatCommand::Plan(rest),
        "goal" => ChatCommand::Goal(rest),
        "status" => ChatCommand::Status,
        "resume" => ChatCommand::Resume(rest),
        "sessions" => ChatCommand::Sessions,
        "diff" => ChatCommand::Diff,
        "apply" => ChatCommand::Apply,
        "approval" => ChatCommand::Approval(rest),
        _ => ChatCommand::Unknown(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_expected_commands_in_display_order() {
        let names: Vec<&str> = SLASH_COMMANDS.iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            [
                "help", "model", "status", "clear", "compact", "init", "plan", "goal", "resume",
                "sessions", "diff", "apply", "approval", "quit"
            ]
        );
    }

    #[test]
    fn every_alias_resolves_to_its_spec() {
        for spec in SLASH_COMMANDS {
            assert_eq!(find_spec(spec.name), Some(spec as &SlashSpec));
            for alias in spec.aliases {
                assert_eq!(find_spec(alias), Some(spec as &SlashSpec));
            }
        }
        assert!(find_spec("nope").is_none());
    }

    #[test]
    fn filter_prefers_exact_then_prefix_in_registry_order() {
        // exact 命中规范名
        assert_eq!(
            filter_slash_commands("model"),
            vec![find_spec("model").unwrap() as &SlashSpec]
        );
        // 别名 exact 命中：q 只应命中 quit，不再重复给出 prefix
        assert_eq!(
            filter_slash_commands("q"),
            vec![find_spec("quit").unwrap() as &SlashSpec]
        );
        // prefix 按注册顺序：status 先于 sessions
        assert_eq!(
            filter_slash_commands("s"),
            vec![
                find_spec("status").unwrap() as &SlashSpec,
                find_spec("sessions").unwrap() as &SlashSpec,
            ]
        );
        // 容忍前导 '/'
        assert_eq!(filter_slash_commands("/mo"), filter_slash_commands("mo"));
        // 空查询返回全量
        assert_eq!(
            filter_slash_commands(""),
            SLASH_COMMANDS.iter().collect::<Vec<&SlashSpec>>()
        );
        // 无命中为空
        assert!(filter_slash_commands("zzz").is_empty());
    }

    #[test]
    fn slash_commands_are_parsed() {
        assert_eq!(parse_command("/quit"), ChatCommand::Quit);
        assert_eq!(parse_command("/exit"), ChatCommand::Quit);
        assert_eq!(parse_command("/q"), ChatCommand::Quit);
        assert_eq!(parse_command("/help"), ChatCommand::Help);
        assert_eq!(parse_command("/h"), ChatCommand::Help);
        assert_eq!(parse_command("/model"), ChatCommand::Model(None));
        assert_eq!(parse_command("/model glm"), ChatCommand::Model(Some("glm")));
        assert_eq!(parse_command("/status"), ChatCommand::Status);
        assert_eq!(parse_command("/clear"), ChatCommand::Clear);
        assert_eq!(parse_command("/new"), ChatCommand::Clear);
        assert_eq!(parse_command("/compact"), ChatCommand::Compact);
        assert_eq!(parse_command("/init"), ChatCommand::Init);
        assert_eq!(parse_command("/plan"), ChatCommand::Plan(None));
        assert_eq!(
            parse_command("/plan 修复登录页"),
            ChatCommand::Plan(Some("修复登录页"))
        );
        assert_eq!(parse_command("/goal"), ChatCommand::Goal(None));
        assert_eq!(
            parse_command("/goal  完成导出功能 "),
            ChatCommand::Goal(Some("完成导出功能"))
        );
        assert_eq!(parse_command("/resume"), ChatCommand::Resume(None));
        assert_eq!(
            parse_command("/resume abc"),
            ChatCommand::Resume(Some("abc"))
        );
        assert_eq!(parse_command("/sessions"), ChatCommand::Sessions);
        assert_eq!(parse_command("/diff"), ChatCommand::Diff);
        assert_eq!(parse_command("/apply"), ChatCommand::Apply);
        assert_eq!(parse_command("/approval"), ChatCommand::Approval(None));
        assert_eq!(
            parse_command("/approval never"),
            ChatCommand::Approval(Some("never"))
        );
        assert_eq!(parse_command("/nope"), ChatCommand::Unknown("/nope"));
    }

    #[test]
    fn plain_text_is_a_message_and_blank_args_are_ignored() {
        assert_eq!(parse_command("修复登录页"), ChatCommand::Message);
        assert_eq!(parse_command("/model   "), ChatCommand::Model(None));
        assert_eq!(
            parse_command("/approval   never"),
            ChatCommand::Approval(Some("never"))
        );
        assert_eq!(parse_command("/resume  "), ChatCommand::Resume(None));
    }

    #[test]
    fn display_includes_arg_hint() {
        assert_eq!(find_spec("model").unwrap().display(), "/model [id]");
        assert_eq!(find_spec("quit").unwrap().display(), "/quit");
    }
}
