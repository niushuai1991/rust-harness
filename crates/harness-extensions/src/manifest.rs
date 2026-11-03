use harness_contracts::{AppError, AppResult};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub description: String,
    pub component: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub contributions: Contributions,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub settings: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub themes: BTreeMap<String, BTreeMap<String, String>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: String,
    pub title: String,
}
pub(crate) fn fail(message: impl ToString) -> AppError {
    AppError::new("plugin", message.to_string())
}
pub(crate) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
impl Manifest {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if !identifier(&self.id) || self.name.trim().is_empty() {
            return Err(fail("invalid plugin id/name"));
        }
        Version::parse(&self.version).map_err(fail)?;
        if !VersionReq::parse(&self.api)
            .map_err(fail)?
            .matches(&Version::new(0, 1, 0))
        {
            return Err(fail("incompatible plugin API"));
        }
        for p in &self.permissions {
            if ![
                "tools",
                "hooks",
                "log",
                "kv",
                "workspace.read",
                "workspace.write",
                "http",
            ]
            .contains(&p.as_str())
            {
                return Err(fail(format!("unknown permission: {p}")));
            }
        }
        for (id, req) in &self.dependencies {
            if !identifier(id) || id == &self.id {
                return Err(fail("invalid dependency"));
            }
            VersionReq::parse(req).map_err(fail)?;
        }
        let mut names = std::collections::HashSet::new();
        for tool in &self.contributions.tools {
            if !identifier(&tool.name) || !names.insert(&tool.name) || !tool.parameters.is_object()
            {
                return Err(fail("invalid/duplicate tool"));
            }
        }
        for command in &self.contributions.commands {
            if !identifier(&command.name) {
                return Err(fail("invalid command"));
            }
        }
        if !self.contributions.tools.is_empty()
            && (self.component.is_none() || !self.permissions.iter().any(|p| p == "tools"))
        {
            return Err(fail(
                "tool contributions require component and tools permission",
            ));
        }
        if self
            .contributions
            .hooks
            .iter()
            .any(|h| !["before_request", "before_tool", "after_tool"].contains(&h.as_str()))
        {
            return Err(fail("unknown hook"));
        }
        if !self.contributions.hooks.is_empty()
            && (self.component.is_none()
                || !self.permissions.iter().any(|p| p == "hooks")
                || !self.permissions.iter().any(|p| p == "tools"))
        {
            return Err(fail("hooks require component, tools and hooks permissions"));
        }
        Ok(())
    }
}
