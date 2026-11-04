//! Small, explicit JSON Schema subset for host-owned forms. No code, HTML or arbitrary CSS.
use crate::{Contributions, PluginManager, manifest::fail};
use harness_contracts::AppResult;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};
pub(crate) fn validate_value(schema: &Value, value: &Value) -> AppResult<()> {
    let valid = match schema["type"].as_str() {
        Some("string") => value.as_str().is_some_and(|s| {
            s.len() <= 65536
                && s.chars().count() >= schema["minLength"].as_u64().unwrap_or(0) as usize
                && s.chars().count() <= schema["maxLength"].as_u64().unwrap_or(65536) as usize
        }),
        Some("number") => value.as_f64().is_some_and(|n| {
            n.is_finite()
                && n >= schema["minimum"].as_f64().unwrap_or(f64::NEG_INFINITY)
                && n <= schema["maximum"].as_f64().unwrap_or(f64::INFINITY)
        }),
        Some("boolean") => value.is_boolean(),
        _ => false,
    };
    if !valid
        || schema
            .get("enum")
            .is_some_and(|e| !e.as_array().is_some_and(|a| a.contains(value)))
    {
        return Err(fail("invalid setting/argument value"));
    }
    Ok(())
}
fn validate_schema(s: &Value) -> AppResult<()> {
    let object = s.as_object().ok_or_else(|| fail("schema must be object"))?;
    if object.keys().any(|k| {
        ![
            "type",
            "title",
            "description",
            "default",
            "enum",
            "minimum",
            "maximum",
            "minLength",
            "maxLength",
        ]
        .contains(&k.as_str())
    }) || ![Some("string"), Some("number"), Some("boolean")].contains(&s["type"].as_str())
    {
        return Err(fail("unsupported form schema"));
    }
    for k in ["minimum", "maximum"] {
        if s.get(k).is_some_and(|v| !v.is_number()) {
            return Err(fail("invalid numeric bound"));
        }
    }
    for k in ["minLength", "maxLength"] {
        if s.get(k)
            .is_some_and(|v| v.as_u64().is_none_or(|n| n > 65536))
        {
            return Err(fail("invalid length bound"));
        }
    }
    if s["minimum"]
        .as_f64()
        .zip(s["maximum"].as_f64())
        .is_some_and(|(a, b)| a > b)
        || s["minLength"]
            .as_u64()
            .zip(s["maxLength"].as_u64())
            .is_some_and(|(a, b)| a > b)
    {
        return Err(fail("inverted bounds"));
    }
    if let Some(e) = s.get("enum") {
        let a = e
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 100)
            .ok_or_else(|| fail("invalid enum"))?;
        let mut base = s.clone();
        base.as_object_mut().unwrap().remove("enum");
        for v in a {
            validate_value(&base, v)?;
        }
    }
    if let Some(v) = s.get("default") {
        validate_value(s, v)?
    }
    Ok(())
}
fn validate_object_schema(s: &Value) -> AppResult<()> {
    if s["type"] != "object"
        || s.as_object().is_none_or(|o| {
            o.keys().any(|k| {
                !["type", "properties", "required", "additionalProperties"].contains(&k.as_str())
            })
        })
        || s.get("additionalProperties").is_some_and(|v| v != false)
    {
        return Err(fail("unsupported command schema"));
    }
    if let Some(p) = s.get("properties") {
        for (k, v) in p
            .as_object()
            .filter(|p| p.len() <= 100)
            .ok_or_else(|| fail("invalid properties"))?
        {
            if !crate::manifest::identifier(k) {
                return Err(fail("invalid parameter name"));
            }
            validate_schema(v)?;
        }
    }
    if let Some(r) = s.get("required") {
        for k in r.as_array().ok_or_else(|| fail("invalid required"))? {
            let key = k.as_str().ok_or_else(|| fail("invalid required name"))?;
            if s["properties"].get(key).is_none() {
                return Err(fail("unknown required property"));
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_arguments(s: &Value, v: &Value) -> AppResult<()> {
    validate_object_schema(s)?;
    let o = v
        .as_object()
        .ok_or_else(|| fail("arguments must be object"))?;
    for (k, v) in o {
        validate_value(
            s["properties"]
                .get(k)
                .ok_or_else(|| fail("unknown argument"))?,
            v,
        )?;
    }
    if let Some(required) = s["required"].as_array() {
        for k in required {
            if !o.contains_key(k.as_str().unwrap()) {
                return Err(fail("missing required argument"));
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_contributions(c: &Contributions) -> AppResult<()> {
    if c.settings.len() > 100 {
        return Err(fail("too many settings"));
    }
    for (k, s) in &c.settings {
        if !crate::manifest::identifier(k) {
            return Err(fail("invalid setting name"));
        }
        validate_schema(s)?
    }
    for command in &c.commands {
        validate_object_schema(&command.parameters)?
    }
    for theme in c.themes.values() {
        for (k, v) in theme {
            let valid = if k == "--font-family" {
                ["system-ui", "sans-serif", "serif", "monospace"].contains(&v.as_str())
            } else {
                [
                    "--bg", "--panel", "--text", "--muted", "--border", "--accent",
                ]
                .contains(&k.as_str())
                    && v.starts_with('#')
                    && [4, 5, 7, 9].contains(&v.len())
                    && v[1..].bytes().all(|b| b.is_ascii_hexdigit())
            };
            if !valid {
                return Err(fail("unsafe/unsupported theme variable"));
            }
        }
    }
    Ok(())
}
impl PluginManager {
    fn kv_path(&self, workspace: &str, id: &str) -> AppResult<PathBuf> {
        let key = workspace
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if key.is_empty() || key.len() > 200 {
            return Err(fail("invalid workspace id"));
        }
        Ok(self.dir.join("kv").join(id).join(format!("{key}.json")))
    }
    fn read_kv(&self, workspace: &str, id: &str) -> AppResult<BTreeMap<String, String>> {
        let p = self.kv_path(workspace, id)?;
        if p.exists() {
            serde_json::from_slice(&fs::read(p).map_err(fail)?).map_err(fail)
        } else {
            Ok(BTreeMap::new())
        }
    }
    pub fn workspace_contributions(&self, workspace: &str) -> AppResult<Vec<Value>> {
        let policies = self.workspace_policies.lock().unwrap();
        let Some(ceiling) = policies.get(workspace) else {
            return Ok(vec![]);
        };
        let state = self.state.lock().unwrap();
        let mut result = vec![];
        for (id, p) in state.plugins.iter().filter(|(_, p)| p.enabled) {
            let can = |permission: &str| {
                p.manifest.permissions.iter().any(|p| p == permission)
                    && p.grants.iter().any(|p| p == permission)
                    && ceiling.iter().any(|p| p == permission)
            };
            let c = &p.manifest.contributions;
            let kv = if can("kv") {
                self.read_kv(workspace, id)?
            } else {
                BTreeMap::new()
            };
            let mut values = serde_json::Map::new();
            if can("kv") {
                for (k, s) in &c.settings {
                    if let Some(v) = kv
                        .get(&format!("settings/{k}"))
                        .and_then(|v| serde_json::from_str::<Value>(v).ok())
                        .filter(|v| validate_value(s, v).is_ok())
                        .or_else(|| s.get("default").cloned())
                    {
                        values.insert(k.clone(), v);
                    }
                }
            }
            result.push(json!({"id":id,"name":p.manifest.name,"settings":if can("kv"){json!(c.settings)}else{json!({})},"values":values,"commands":if can("tools"){json!(c.commands)}else{json!([])},"themes":c.themes}));
        }
        Ok(result)
    }
    pub async fn save_settings(&self, workspace: &str, id: &str, values: Value) -> AppResult<()> {
        let _serial = self.serial.lock().await;
        let policies = self.workspace_policies.lock().unwrap();
        if !policies
            .get(workspace)
            .is_some_and(|p| p.iter().any(|p| p == "kv"))
        {
            return Err(fail("workspace kv permission denied"));
        }
        let state = self.state.lock().unwrap();
        let p = state
            .plugins
            .get(id)
            .filter(|p| p.enabled && p.grants.iter().any(|p| p == "kv"))
            .ok_or_else(|| fail("plugin disabled/kv permission denied"))?;
        let values = values
            .as_object()
            .ok_or_else(|| fail("settings must be object"))?;
        let mut kv = self.read_kv(workspace, id)?;
        for (k, v) in values {
            let schema = p
                .manifest
                .contributions
                .settings
                .get(k)
                .ok_or_else(|| fail("unknown setting"))?;
            validate_value(schema, v)?;
            kv.insert(format!("settings/{k}"), v.to_string());
        }
        if kv.len() > 256 {
            return Err(fail("KV limit"));
        }
        let path = self.kv_path(workspace, id)?;
        fs::create_dir_all(path.parent().unwrap()).map_err(fail)?;
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&tmp, serde_json::to_vec(&kv).map_err(fail)?).map_err(fail)?;
        fs::rename(tmp, path).map_err(fail)
    }
    /// Child cancellation scope: disabling the plugin also interrupts a pending human approval.
    pub fn command_cancellation(&self, id: &str) -> AppResult<tokio_util::sync::CancellationToken> {
        let state = self.state.lock().unwrap();
        if !state.plugins.get(id).is_some_and(|p| p.enabled) {
            return Err(fail("plugin disabled"));
        }
        Ok(state.tokens[id].child_token())
    }
    pub async fn execute_command(
        &self,
        workspace: &str,
        root: &std::path::Path,
        id: &str,
        name: &str,
        args: Value,
        cancel: tokio_util::sync::CancellationToken,
    ) -> AppResult<harness_contracts::ToolResult> {
        if !crate::manifest::identifier(id) || !crate::manifest::identifier(name) {
            return Err(fail("invalid command"));
        }
        self.execute_invocation(
            workspace,
            root,
            &format!("plugin__{id}__command:{name}"),
            args,
            cancel,
            true,
        )
        .await
    }
}
