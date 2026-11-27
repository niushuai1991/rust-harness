//! Agent Hook configuration validation.
use harness_contracts::{AppError, AppResult};
use serde_json::{Value, json};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) struct HookRun {
    pub status: &'static str,
    pub context: Option<String>,
    pub diagnostic: Option<String>,
    pub blocked: bool,
    pub updated_input: Option<Value>,
    pub ask: bool,
}

type PluginSnapshot = (String, std::path::PathBuf, Vec<u8>);
type HookGroup = (Value, Option<String>, Option<PluginSnapshot>);

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("HOOK_CONFIG", message)
}

/// 项目目录模板变量：`${RH_PROJECT_DIR}` 与 Claude Code 兼容别名 `${CLAUDE_PROJECT_DIR}`，
/// 均展开为当前工作区（hook 执行根）。变量是运行时值，不参与信任摘要。
fn expand_project_variables(value: &str, root: &Path) -> String {
    let project_dir = root.to_string_lossy();
    value
        .replace("${RH_PROJECT_DIR}", &project_dir)
        .replace("${CLAUDE_PROJECT_DIR}", &project_dir)
}

/// Validate only the hooks section, preserving unrelated settings.
pub(crate) fn validate_settings(data_dir: &Path) -> AppResult<()> {
    let Some(value) = read_settings(data_dir)? else {
        return Ok(());
    };
    validate_value(&value)
}

fn read_settings(data_dir: &Path) -> AppResult<Option<Value>> {
    let path = data_dir.join("settings.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(invalid(format!("无法读取 {}：{error}", path.display()))),
    };
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("{} 不是有效 JSON：{error}", path.display())))?;
    Ok(Some(value))
}

pub(crate) fn validate_value(value: &Value) -> AppResult<()> {
    let Some(hooks) = value.get("hooks") else {
        return Ok(());
    };
    let events = hooks
        .as_object()
        .ok_or_else(|| invalid("hooks 必须是对象"))?;
    let mut unsupported_handlers = false;
    for (event, groups) in events {
        if !matches!(
            event.as_str(),
            "SessionStart"
                | "UserPromptSubmit"
                | "PreToolUse"
                | "PermissionRequest"
                | "PostToolUse"
                | "PostToolUseFailure"
                | "Stop"
                | "Notification"
                | "PostToolBatch"
        ) {
            return Err(invalid(format!("不支持的 Hook 事件：{event}")));
        }
        for group in groups
            .as_array()
            .ok_or_else(|| invalid(format!("{event} 必须是数组")))?
        {
            let group = group
                .as_object()
                .ok_or_else(|| invalid("Hook 匹配组必须是对象"))?;
            if group.get("matcher").is_some_and(|v| !v.is_string()) {
                return Err(invalid("matcher 必须是字符串"));
            }
            if event == "SessionStart"
                && let Some(matcher) = group.get("matcher").and_then(Value::as_str)
                && matcher != "*"
                && !matcher
                    .split('|')
                    .all(|part| matches!(part.trim(), "" | "startup" | "resume"))
            {
                return Err(invalid("暂仅支持 SessionStart 的 startup 和 resume 匹配"));
            }
            if matches!(
                event.as_str(),
                "PreToolUse" | "PermissionRequest" | "PostToolUse" | "PostToolUseFailure"
            ) && let Some(matcher) = group.get("matcher").and_then(Value::as_str)
                && matcher != "*"
                && !matcher
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '|' | ',' | ' '))
            {
                return Err(invalid("PreToolUse 正则 matcher 暂不支持"));
            }
            let handlers = group
                .get("hooks")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Hook 匹配组必须包含 hooks 数组"))?;
            for handler in handlers {
                unsupported_handlers |= !matches!(
                    event.as_str(),
                    "SessionStart"
                        | "UserPromptSubmit"
                        | "PreToolUse"
                        | "PermissionRequest"
                        | "Notification"
                        | "PostToolUse"
                        | "PostToolUseFailure"
                        | "Stop"
                        | "PostToolBatch"
                );
                if handler.get("type").and_then(Value::as_str) != Some("command") {
                    return Err(invalid("仅支持 command 类型的 Hook"));
                }
                if handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_none_or(|s| s.trim().is_empty())
                {
                    return Err(invalid("Hook command 不能为空"));
                }
                let object = handler
                    .as_object()
                    .ok_or_else(|| invalid("Hook 处理器必须是对象"))?;
                if object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "type" | "command" | "args" | "timeout" | "shell"
                    )
                }) {
                    return Err(invalid("Hook 含暂不支持的处理器字段"));
                }
                if let Some(args) = handler.get("args")
                    && args
                        .as_array()
                        .is_none_or(|args| args.iter().any(|arg| !arg.is_string()))
                {
                    return Err(invalid("Hook args 必须是字符串数组"));
                }
                if let Some(timeout) = handler.get("timeout")
                    && timeout.as_u64().is_none_or(|n| !(1..=600).contains(&n))
                {
                    return Err(invalid("Hook timeout 须为 1 至 600 秒"));
                }
                if let Some(shell) = handler.get("shell")
                    && shell.as_str() != Some("powershell")
                {
                    return Err(invalid("暂仅支持 powershell shell"));
                }
            }
        }
    }
    if unsupported_handlers {
        return Err(AppError::new(
            "HOOK_UNAVAILABLE",
            "Agent Hook 执行功能尚未实现，无法安全继续",
        ));
    }
    Ok(())
}

/// Executes a supported event after validating the same configuration snapshot.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_event(
    store: &harness_storage::Store,
    plugins: &harness_extensions::PluginManager,
    data_dir: &Path,
    root: &Path,
    capability_id: &str,
    sandboxed: bool,
    session_id: &str,
    event: &str,
    mut payload: Value,
    cancel: CancellationToken,
) -> AppResult<Vec<HookRun>> {
    let mut groups: Vec<HookGroup> = Vec::new();
    if let Some(settings) = read_settings(data_dir)? {
        validate_value(&settings)?;
        if let Some(global) = settings["hooks"][event].as_array() {
            groups.extend(global.iter().cloned().map(|group| (group, None, None)));
        }
    }
    if groups.is_empty()
        && matches!(std::fs::symlink_metadata(root.join(".rh")), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        && !plugins.has_command_hook_grant()
    {
        return Ok(Vec::new());
    }
    let trust = crate::hook_trust::HookTrustService::new(store.clone());
    let workspace_id = store.session(session_id).await?.workspace_id;
    if !workspace_id.is_empty() {
        let reviews = trust.review(root).await?;
        if reviews.iter().any(|entry| !entry.trusted) {
            return Err(AppError::new("HOOK_TRUST_REQUIRED", "项目 Hook 需要审核"));
        }
        groups.extend(reviews.into_iter().filter(|entry| entry.event == event).map(|entry| {
            (json!({"matcher":entry.matcher.unwrap_or_default(), "hooks":[entry.handler]}), Some(entry.digest), None)
        }));
    }
    for (id, path) in plugins.command_hook_sources(&workspace_id)? {
        let bytes = std::fs::read(&path).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("插件 Hook 配置超过 1 MiB"));
        }
        let settings: Value = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
        validate_value(&settings)?;
        if let Some(plugin_groups) = settings["hooks"][event].as_array() {
            let plugin_root = path
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| invalid("插件路径无效"))?
                .to_string_lossy();
            for group in plugin_groups {
                let mut group = group.clone();
                for handler in group["hooks"].as_array_mut().into_iter().flatten() {
                    if let Some(command) = handler["command"].as_str() {
                        handler["command"] =
                            Value::String(command.replace("${RH_PLUGIN_ROOT}", &plugin_root));
                    }
                    if let Some(args) = handler.get_mut("args").and_then(Value::as_array_mut) {
                        for arg in args {
                            if let Some(value) = arg.as_str() {
                                *arg =
                                    Value::String(value.replace("${RH_PLUGIN_ROOT}", &plugin_root));
                            }
                        }
                    }
                }
                groups.push((group, None, Some((id.clone(), path.clone(), bytes.clone()))));
            }
        }
    }
    let mut results = Vec::new();
    for (group, digest, plugin) in &groups {
        let matcher = group["matcher"].as_str().unwrap_or("");
        let source = payload["source"].as_str().unwrap_or("");
        if event == "SessionStart"
            && !matcher.is_empty()
            && matcher != "*"
            && !matcher.split('|').any(|part| part.trim() == source)
        {
            continue;
        }
        let tool_name = payload["tool_name"].as_str().unwrap_or("");
        if event == "Notification"
            && !matcher.is_empty()
            && matcher != "*"
            && !matcher
                .split(['|', ','])
                .any(|part| part.trim() == payload["notification_type"].as_str().unwrap_or(""))
        {
            continue;
        }
        if matches!(
            event,
            "PreToolUse" | "PermissionRequest" | "PostToolUse" | "PostToolUseFailure"
        ) && !matcher.is_empty()
            && matcher != "*"
            && !matcher
                .split(['|', ','])
                .any(|part| part.trim() == tool_name)
        {
            continue;
        }
        for handler in group["hooks"].as_array().into_iter().flatten() {
            if !workspace_id.is_empty() {
                let current = trust.review(root).await?;
                if current.iter().any(|entry| !entry.trusted)
                    || digest.as_ref().is_some_and(|digest| {
                        !current
                            .iter()
                            .any(|entry| entry.digest == *digest && entry.trusted)
                    })
                {
                    return Err(AppError::new(
                        "HOOK_TRUST_REQUIRED",
                        "项目 Hook 已更改或信任已撤销，请重新审核",
                    ));
                }
            }
            if let Some((id, path, bytes)) = plugin
                && (!plugins
                    .command_hook_sources(&workspace_id)?
                    .iter()
                    .any(|source| &source.0 == id && &source.1 == path)
                    || std::fs::read(path).map_err(|e| invalid(e.to_string()))? != *bytes)
            {
                return Err(invalid("插件 Hook 授权或配置已变化"));
            }
            let command = handler["command"]
                .as_str()
                .ok_or_else(|| invalid("缺少 command"))?;
            let (program, args) = if let Some(args) = handler.get("args") {
                (
                    command.to_owned(),
                    args.as_array()
                        .ok_or_else(|| invalid("args 不是数组"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| invalid("args 包含非字符串"))
                        })
                        .collect::<AppResult<Vec<_>>>()?,
                )
            } else {
                (
                    "powershell.exe".to_owned(),
                    vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        command.to_owned(),
                    ],
                )
            };
            let mut input = json!({"session_id":session_id,"cwd":root,"hook_event_name":event});
            if let (Some(target), Some(extra)) = (input.as_object_mut(), payload.as_object()) {
                target.extend(extra.clone());
            }
            let mut input =
                serde_json::to_vec(&input).map_err(|error| invalid(error.to_string()))?;
            input.push(b'\n');
            let program = expand_project_variables(&program, root);
            let args = args
                .iter()
                .map(|arg| expand_project_variables(arg, root))
                .collect::<Vec<_>>();
            let project_dir = root.to_string_lossy().into_owned();
            let env = std::collections::BTreeMap::from([
                ("RH_PROJECT_DIR".to_owned(), project_dir.clone()),
                ("CLAUDE_PROJECT_DIR".to_owned(), project_dir),
            ]);
            let timeout = handler["timeout"].as_u64().unwrap_or(60);
            match harness_platform::ShellService::run_hook_in(
                root,
                capability_id,
                &program,
                &args,
                &input,
                &env,
                cancel.clone(),
                sandboxed,
                std::time::Duration::from_secs(timeout),
            )
            .await
            {
                Ok(output) if output.exit_code == 0 => {
                    let text = output.stdout.trim();
                    let structured = if text.starts_with('{') {
                        serde_json::from_str::<Value>(text).ok()
                    } else {
                        None
                    };
                    let context = if let Some(value) = &structured {
                        value["hookSpecificOutput"]["additionalContext"]
                            .as_str()
                            .map(str::to_owned)
                    } else if text.is_empty()
                        || text.starts_with('{')
                        || !matches!(event, "SessionStart" | "UserPromptSubmit")
                    {
                        None
                    } else {
                        Some(text.to_owned())
                    };
                    let blocked = structured.as_ref().is_some_and(|value| {
                        value["decision"] == "block"
                            || value["continue"] == false
                            || value["hookSpecificOutput"]["permissionDecision"] == "deny"
                    }) || (event == "Stop" && context.is_some())
                        || (event == "PermissionRequest"
                            && structured.as_ref().is_some_and(|value| {
                                value["hookSpecificOutput"]["decision"]["behavior"] == "deny"
                            }));
                    let diagnostic = structured
                        .as_ref()
                        .and_then(|value| {
                            value["hookSpecificOutput"]["decision"]["message"]
                                .as_str()
                                .or_else(|| {
                                    value["hookSpecificOutput"]["permissionDecisionReason"].as_str()
                                })
                                .or_else(|| value["reason"].as_str())
                        })
                        .map(str::to_owned);
                    if event == "PreToolUse"
                        && let Some(value) = &structured
                        && let Some(decision) =
                            value["hookSpecificOutput"].get("permissionDecision")
                        && !matches!(decision.as_str(), Some("allow" | "deny" | "ask"))
                    {
                        return Err(invalid("Hook permissionDecision 无效"));
                    }
                    let updated_input = if event == "PreToolUse" {
                        structured
                            .as_ref()
                            .and_then(|value| value["hookSpecificOutput"].get("updatedInput"))
                            .cloned()
                    } else {
                        None
                    };
                    let ask = event == "PreToolUse"
                        && structured.as_ref().is_some_and(|value| {
                            value["hookSpecificOutput"]["permissionDecision"] == "ask"
                        });
                    if let Some(value) = &updated_input {
                        if !value.is_object() {
                            return Err(invalid("Hook updatedInput 必须是对象"));
                        }
                        payload["tool_input"] = value.clone();
                    }
                    results.push(HookRun {
                        status: if blocked { "denied" } else { "completed" },
                        context,
                        diagnostic,
                        blocked,
                        updated_input,
                        ask,
                    });
                }
                Ok(output) => results.push(HookRun {
                    status: if output.exit_code == 2
                        && matches!(event, "UserPromptSubmit" | "PreToolUse" | "Stop")
                    {
                        "denied"
                    } else {
                        "error"
                    },
                    context: None,
                    diagnostic: Some(
                        if output.exit_code == 2
                            && matches!(event, "UserPromptSubmit" | "PreToolUse" | "Stop")
                        {
                            output.stderr
                        } else {
                            format!("Hook 退出码 {}", output.exit_code)
                        },
                    ),
                    blocked: output.exit_code == 2
                        && matches!(event, "UserPromptSubmit" | "PreToolUse" | "Stop"),
                    updated_input: None,
                    ask: false,
                }),
                Err(error) if error.code == "CANCELLED" => return Err(error),
                Err(error) => results.push(HookRun {
                    status: "error",
                    context: None,
                    diagnostic: Some(error.to_string()),
                    blocked: false,
                    updated_input: None,
                    ask: false,
                }),
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod expand_tests {
    use super::*;

    #[test]
    fn expands_both_aliases_to_the_workspace_root() {
        let root = Path::new("D:/work/proj");
        assert_eq!(
            expand_project_variables("${RH_PROJECT_DIR}", root),
            "D:/work/proj"
        );
        assert_eq!(
            expand_project_variables("${CLAUDE_PROJECT_DIR}", root),
            "D:/work/proj"
        );
    }

    #[test]
    fn replaces_every_occurrence_and_keeps_other_text_intact() {
        let root = Path::new("C:/tmp/r");
        assert_eq!(
            expand_project_variables(
                "bash \"${CLAUDE_PROJECT_DIR}/a.sh\" \"${RH_PROJECT_DIR}/b.sh\" ${CLAUDE_PROJECT_DIR}",
                root,
            ),
            "bash \"C:/tmp/r/a.sh\" \"C:/tmp/r/b.sh\" C:/tmp/r"
        );
    }

    #[test]
    fn leaves_unknown_variables_untouched() {
        let root = Path::new("C:/tmp/r");
        assert_eq!(
            expand_project_variables("${RH_PLUGIN_ROOT} ${OTHER}", root),
            "${RH_PLUGIN_ROOT} ${OTHER}"
        );
    }
}
