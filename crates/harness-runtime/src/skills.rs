//! Skills（技能）：SKILL.md 目录发现、frontmatter 解析、catalog 注入与 `$name` 提及
//! 解析（docs/specs/skills.md）。设计参考 codex skills 体系，按本项目体量简化。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use harness_contracts::{AppError, AppResult, SkillInfo, SkillScope};
use harness_storage::Store;

/// 禁用名单 settings 键（JSON 字符串数组，元素为 SkillInfo.path）。
const DISABLED_KEY: &str = "skills.disabled";
/// catalog 总量截断上限（字符）。
const MAX_CATALOG_CHARS: usize = 8000;
/// 单条 description 截断上限（字符）。
const MAX_DESCRIPTION_CHARS: usize = 200;
/// `$name` 注入正文的字节截断上限（风格同 AGENTS.md 的 32000 字符）。
const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;
/// 源文件读取上限（防巨型文件），剥离 frontmatter 后再按正文上限截断。
const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// 兜底扫描的文件数上限（与既有 workspace_context 行为一致）。
const MAX_SCAN_FILES: usize = 100;
/// name 合法性：长度上限。
const MAX_NAME_LEN: usize = 64;

#[derive(Clone)]
pub struct SkillService {
    store: Store,
    gate: Arc<Mutex<()>>,
    /// 用户级扫描根（注入用）；缺省为真实主目录。
    home: Option<PathBuf>,
}

/// 解析命中的技能：目录行 + SKILL.md 正文（已按上限截断）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSkill {
    pub info: SkillInfo,
    pub content: String,
}

/// frontmatter 解析结果；字段缺失为 None（fail-open，不判 broken）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
}

impl SkillService {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            gate: Arc::new(Mutex::new(())),
            home: home_dir(),
        }
    }

    /// home 可注入形式（测试隔离用户级目录用）。
    #[cfg(test)]
    fn with_home(store: Store, home: Option<PathBuf>) -> Self {
        Self {
            store,
            gate: Arc::new(Mutex::new(())),
            home,
        }
    }

    /// 全量目录：三源扫描 + 禁用标记；排序 project 主约定 > project 兜底 > user，同级按 name。
    pub async fn list(&self, root: &Path) -> AppResult<Vec<SkillInfo>> {
        let disabled = self.disabled().await?;
        Ok(discover_with_home(root, self.home.as_deref())
            .into_iter()
            .map(|mut info| {
                info.enabled = !disabled.contains(&info.path);
                info
            })
            .collect())
    }

    /// 系统提示注入的 catalog 文本；无可用技能返回空串。
    pub async fn catalog(&self, root: &Path) -> AppResult<String> {
        let disabled = self.disabled().await?;
        let usable: Vec<SkillInfo> = discover_with_home(root, self.home.as_deref())
            .into_iter()
            .filter(|s| !disabled.contains(&s.path) && s.broken_reason.is_none())
            .collect();
        Ok(render_catalog(&usable))
    }

    /// 解析 `$name` 提及：按排序取第一同名可用技能；未命中返回 None。
    pub async fn resolve(&self, root: &Path, name: &str) -> AppResult<Option<ResolvedSkill>> {
        let disabled = self.disabled().await?;
        let Some(info) = discover_with_home(root, self.home.as_deref())
            .into_iter()
            .find(|s| s.name == name && !disabled.contains(&s.path) && s.broken_reason.is_none())
        else {
            return Ok(None);
        };
        let content = match info.scope {
            SkillScope::User => {
                let Some(home) = self.home.clone() else {
                    return Ok(None);
                };
                read_user_skill(&home.join(&info.path))?
            }
            SkillScope::Project => {
                harness_platform::read_file_bounded(root, &info.path, MAX_SOURCE_BYTES)?
            }
        };
        Ok(Some(ResolvedSkill {
            info,
            content: finalize_content(&content),
        }))
    }

    /// 启停（按 path 写禁用名单，幂等）。
    pub async fn set_enabled(&self, path: &str, enabled: bool) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        let mut all = self.disabled().await?;
        if enabled {
            all.remove(path);
        } else {
            all.insert(path.to_string());
        }
        self.store
            .set(DISABLED_KEY, &json!(all.into_iter().collect::<Vec<_>>()))
            .await
    }

    async fn disabled(&self) -> AppResult<BTreeSet<String>> {
        serde_json::from_value::<Vec<String>>(
            self.store.get(DISABLED_KEY).await?.unwrap_or(json!([])),
        )
        .map_err(|e| AppError::new("SKILL_CONFIG", e.to_string()))
        .map(|list| list.into_iter().collect())
    }
}

/// 从工作副本（项目级主约定 + 兜底）与用户主目录发现技能；不含禁用标记。
pub fn discover(root: &Path) -> Vec<SkillInfo> {
    discover_with_home(root, home_dir().as_deref())
}

/// `discover` 的可注入形式（测试用）。
pub fn discover_with_home(root: &Path, home: Option<&Path>) -> Vec<SkillInfo> {
    let mut out = Vec::new();
    if let Ok(files) = harness_platform::list_files(root) {
        for relative in files
            .into_iter()
            .filter(|f| is_skill_file(f))
            .take(MAX_SCAN_FILES)
        {
            out.push(build_skill(
                root,
                &relative,
                SkillScope::Project,
                relative.clone(),
            ));
        }
    }
    if let Some(home) = home {
        let skills_root = home.join(".agents").join("skills");
        if let Ok(files) = harness_platform::list_files(&skills_root) {
            for relative in files
                .into_iter()
                .filter(|f| is_skill_file(f))
                .take(MAX_SCAN_FILES)
            {
                let absolute = skills_root.join(&relative);
                let display = absolute.to_string_lossy().replace('\\', "/");
                out.push(build_skill(
                    &skills_root,
                    &relative,
                    SkillScope::User,
                    display,
                ));
            }
        }
    }
    out.sort_by(|a, b| scope_key(a).cmp(&scope_key(b)));
    out
}

/// 排序键：项目级主约定 < 项目级兜底 < 用户级，同级按 name、path。
fn scope_key(info: &SkillInfo) -> (u8, &str, &str) {
    let rank = match info.scope {
        SkillScope::User => 2,
        SkillScope::Project if info.path.starts_with(".agents/skills/") => 0,
        SkillScope::Project => 1,
    };
    (rank, &info.name, &info.path)
}

/// 组装目录行：读取 frontmatter，读取失败或 name 非法时 broken（fail-open，不阻塞其余技能）。
fn build_skill(base: &Path, relative: &str, scope: SkillScope, display: String) -> SkillInfo {
    let broken = |reason: String| SkillInfo {
        name: default_skill_name(relative),
        description: None,
        path: display.clone(),
        scope,
        enabled: true,
        broken_reason: Some(reason),
    };
    let text = match harness_platform::read_file(base, relative) {
        Ok(text) => text,
        Err(e) => return broken(format!("SKILL.md 读取失败：{e}")),
    };
    let parsed = parse_frontmatter(&text);
    let name = parsed.name.unwrap_or_else(|| default_skill_name(relative));
    if !valid_name(&name) {
        return broken(format!(
            "技能名 {name} 非法（限 [A-Za-z0-9_-]，不超过 {MAX_NAME_LEN} 字符）"
        ));
    }
    SkillInfo {
        name,
        description: parsed
            .description
            .map(|d| d.chars().take(MAX_DESCRIPTION_CHARS).collect()),
        path: display,
        scope,
        enabled: true,
        broken_reason: None,
    }
}

/// name 缺省取父目录名（`.agents/skills/review/SKILL.md` → `review`）。
fn default_skill_name(relative: &str) -> String {
    relative.rsplit('/').nth(1).unwrap_or("skill").to_string()
}

/// 解析 SKILL.md frontmatter：`---` 围栏内逐行 `key: value`，只认 name/description，容忍未知字段。
pub fn parse_frontmatter(text: &str) -> ParsedFrontmatter {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return ParsedFrontmatter::default();
    }
    let mut parsed = ParsedFrontmatter::default();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        match key.trim() {
            "name" if !value.is_empty() => parsed.name = Some(value.to_string()),
            "description" if !value.is_empty() => parsed.description = Some(value.to_string()),
            _ => {}
        }
    }
    parsed
}

/// 提取用户消息中的 `$name` 提及：仅命中已知技能名单的名字才算（保序去重）。
pub fn extract_mentions(text: &str, known: &BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, ch) in text.char_indices() {
        if ch != '$' {
            continue;
        }
        let start = i + 1;
        let end = text[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .map_or(text.len(), |offset| start + offset);
        if end == start {
            continue;
        }
        let name = &text[start..end];
        if known.contains(name) && !out.iter().any(|x| x == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// 渲染 catalog；空列表返回空串。仅接收启用且未 broken 的技能。
pub fn render_catalog(skills: &[SkillInfo]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("## 可用技能\n");
    for skill in skills {
        let line = format!(
            "- {}: {} ({})\n",
            skill.name,
            skill.description.as_deref().unwrap_or("（无描述）"),
            skill.path
        );
        if out.chars().count() + line.chars().count() > MAX_CATALOG_CHARS {
            out.push_str("（其余技能省略）\n");
            return out;
        }
        out.push_str(&line);
    }
    out.push_str(
        "\n使用规则：用户消息以 $name 提及某技能、或任务与上述描述明确匹配时，读取对应 SKILL.md 并遵循其内容执行。技能只在被提及或匹配的回合使用。\n",
    );
    out
}

/// name 合法性：非空、≤64 字符、仅 `[A-Za-z0-9_-]`。
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 用户主目录（`USERPROFILE`/`HOME`），缺失返回 None。
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// 读取 SKILL.md 源文件（含 frontmatter），按源文件上限截断；UTF-8 尾部损坏由 lossy 处理。
fn read_user_skill(absolute: &Path) -> AppResult<String> {
    use std::io::Read;
    let file =
        std::fs::File::open(absolute).map_err(|e| AppError::new("SKILL_IO", e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::new("SKILL_IO", e.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 注入正文：剥离 frontmatter（name/description 已在 catalog 与 `<skill>` 标记中），正文按 32KiB 截断。
fn finalize_content(raw: &str) -> String {
    truncate_bytes(strip_frontmatter(raw), MAX_SKILL_BODY_BYTES)
}

/// 剥离 frontmatter 围栏返回正文；无围栏或无闭合围栏时按原文返回（fail-open）。
pub fn strip_frontmatter(text: &str) -> &str {
    if text.lines().next().map(str::trim) != Some("---") {
        return text;
    }
    let mut rest = &text[text.find('\n').map_or(text.len(), |i| i + 1)..];
    while !rest.is_empty() {
        let (line, tail) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        if line.trim() == "---" {
            return tail.trim_start();
        }
        rest = tail;
    }
    text
}

/// 按字节数截断，不切断 UTF-8 字符。
fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// SKILL.md 文件名匹配（大小写不敏感，同 codex）。
fn is_skill_file(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("SKILL.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn skill(path: &str, name: &str, description: Option<&str>, scope: SkillScope) -> SkillInfo {
        SkillInfo {
            name: name.into(),
            description: description.map(str::to_owned),
            path: path.into(),
            scope,
            enabled: true,
            broken_reason: None,
        }
    }

    #[test]
    fn frontmatter_parses_name_and_description() {
        let text = "---\nname: review\ndescription: 代码审查技能\n---\n\n正文内容。\n";
        let parsed = parse_frontmatter(text);
        assert_eq!(parsed.name.as_deref(), Some("review"));
        assert_eq!(parsed.description.as_deref(), Some("代码审查技能"));
    }

    #[test]
    fn frontmatter_without_fence_yields_none() {
        let parsed = parse_frontmatter("# 没有围栏的技能\n\n正文。\n");
        assert_eq!(parsed, ParsedFrontmatter::default());
    }

    #[test]
    fn frontmatter_tolerates_unknown_fields_and_colons() {
        let text = "---\nname: deploy\nargument-hint: <env: e.g. prod>\ntags: [a, b]\ndescription: 部署: 生产环境\n---\nbody";
        let parsed = parse_frontmatter(text);
        assert_eq!(parsed.name.as_deref(), Some("deploy"));
        assert_eq!(parsed.description.as_deref(), Some("部署: 生产环境"));
    }

    #[test]
    fn frontmatter_missing_description_is_not_broken_marker() {
        let parsed = parse_frontmatter("---\nname: x\n---\nbody");
        assert_eq!(parsed.name.as_deref(), Some("x"));
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn mentions_require_known_names_and_dedupe_in_order() {
        let known: BTreeSet<String> = ["review", "deploy", "fix-docs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let text = "先用 $review 审查，再 $deploy；$PATH 与 $unknown 忽略，$review 不重复。";
        assert_eq!(
            extract_mentions(text, &known),
            vec!["review".to_string(), "deploy".to_string()]
        );
    }

    #[test]
    fn mentions_respect_token_boundaries() {
        let known: BTreeSet<String> = ["deploy".to_string()].into_iter().collect();
        assert_eq!(extract_mentions("$deploy ", &known), vec!["deploy"]);
        assert_eq!(extract_mentions("$deploy-x ", &known), Vec::<String>::new());
        assert_eq!(extract_mentions("$", &known), Vec::<String>::new());
        assert_eq!(extract_mentions("邮箱 a$deploy b", &known), vec!["deploy"]);
    }

    #[test]
    fn catalog_renders_lines_and_rules() {
        let skills = vec![
            skill(
                ".agents/skills/review/SKILL.md",
                "review",
                Some("代码审查"),
                SkillScope::Project,
            ),
            skill(
                "C:/h/.agents/skills/deploy/SKILL.md",
                "deploy",
                None,
                SkillScope::User,
            ),
        ];
        let text = render_catalog(&skills);
        assert!(text.starts_with("## 可用技能\n"));
        assert!(text.contains("- review: 代码审查 (.agents/skills/review/SKILL.md)"));
        assert!(text.contains("- deploy: （无描述） (C:/h/.agents/skills/deploy/SKILL.md)"));
        assert!(text.contains("使用规则"));
    }

    #[test]
    fn catalog_empty_yields_empty_string() {
        assert_eq!(render_catalog(&[]), "");
    }

    #[test]
    fn catalog_truncates_over_budget() {
        let skills: Vec<SkillInfo> = (0..400)
            .map(|i| {
                skill(
                    &format!(".agents/skills/s{i:03}/SKILL.md"),
                    &format!("s{i:03}"),
                    Some("很长的描述"),
                    SkillScope::Project,
                )
            })
            .collect();
        let text = render_catalog(&skills);
        assert!(text.chars().count() <= MAX_CATALOG_CHARS + 20);
        assert!(text.contains("（其余技能省略）"));
    }

    #[test]
    fn name_rules() {
        assert!(valid_name("review"));
        assert!(valid_name("fix-docs_2"));
        assert!(!valid_name(""));
        assert!(!valid_name("有空格"));
        assert!(!valid_name(&"x".repeat(MAX_NAME_LEN + 1)));
    }

    #[test]
    fn discover_reads_project_convention_fallback_and_user_dir() {
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            ".agents/skills/review/SKILL.md",
            "---\ndescription: 代码审查\n---\n正文 review",
        );
        write(
            repo.path(),
            ".skills/p1/SKILL.md",
            "---\nname: p1\ndescription: P1 兜底\n---\n正文 p1",
        );
        write(
            home.path(),
            ".agents/skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: 用户级部署\n---\n正文 deploy",
        );
        let skills = discover_with_home(repo.path(), Some(home.path()));
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["review", "p1", "deploy"],
            "主约定 > 兜底 > 用户级"
        );
        assert_eq!(skills[0].scope, SkillScope::Project);
        assert_eq!(skills[0].description.as_deref(), Some("代码审查"));
        assert_eq!(skills[2].scope, SkillScope::User);
        assert!(
            skills
                .iter()
                .all(|s| s.enabled && s.broken_reason.is_none())
        );
    }

    #[test]
    fn discover_defaults_name_to_dir_and_flags_invalid() {
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), ".agents/skills/review/SKILL.md", "正文无围栏");
        write(
            repo.path(),
            ".agents/skills/bad name/SKILL.md",
            "---\nname: bad name\n---\n正文",
        );
        let skills = discover_with_home(repo.path(), None);
        let by_name = |n: &str| {
            skills
                .iter()
                .find(|s| s.path.contains(n))
                .expect("skill present")
        };
        assert_eq!(by_name("review").name, "review", "name 缺省取父目录名");
        assert!(
            by_name("bad name").broken_reason.is_some(),
            "非法 name 判 broken"
        );
    }

    #[test]
    fn discover_truncates_long_description() {
        let repo = tempfile::tempdir().unwrap();
        let long = "描".repeat(300);
        write(
            repo.path(),
            ".agents/skills/review/SKILL.md",
            &format!("---\ndescription: {long}\n---\n正文"),
        );
        let skills = discover_with_home(repo.path(), None);
        assert_eq!(
            skills[0].description.as_deref().map(|d| d.chars().count()),
            Some(MAX_DESCRIPTION_CHARS)
        );
    }

    /// 隔离 home 的 service 测试助手（用户级目录不扫真实主目录）。
    async fn service() -> SkillService {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("config.db")).await.unwrap();
        SkillService::with_home(store, None)
    }

    #[tokio::test]
    async fn service_lists_marks_disabled_and_catalog_respects_it() {
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            ".agents/skills/review/SKILL.md",
            "---\ndescription: 代码审查\n---\n正文",
        );
        let s = service().await;

        let list = s.list(repo.path()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].enabled);

        s.set_enabled(&list[0].path, false).await.unwrap();
        let list = s.list(repo.path()).await.unwrap();
        assert!(!list[0].enabled);
        assert_eq!(s.catalog(repo.path()).await.unwrap(), "");

        s.set_enabled(&list[0].path, true).await.unwrap();
        assert!(s.list(repo.path()).await.unwrap()[0].enabled);
        assert!(s.catalog(repo.path()).await.unwrap().contains("review"));
    }

    #[tokio::test]
    async fn service_resolve_returns_first_match_and_none_when_disabled() {
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            ".agents/skills/review/SKILL.md",
            "---\ndescription: 主约定\n---\n主约定正文",
        );
        write(
            repo.path(),
            ".skills/review/SKILL.md",
            "---\ndescription: 兜底\n---\n兜底正文",
        );
        let s = service().await;

        let resolved = s.resolve(repo.path(), "review").await.unwrap().unwrap();
        assert_eq!(resolved.info.path, ".agents/skills/review/SKILL.md");
        assert_eq!(resolved.content, "主约定正文");

        s.set_enabled(".agents/skills/review/SKILL.md", false)
            .await
            .unwrap();
        let resolved = s.resolve(repo.path(), "review").await.unwrap().unwrap();
        assert_eq!(
            resolved.info.path, ".skills/review/SKILL.md",
            "禁用后落到下一条同名"
        );

        s.set_enabled(".skills/review/SKILL.md", false)
            .await
            .unwrap();
        assert!(s.resolve(repo.path(), "review").await.unwrap().is_none());
        assert!(s.resolve(repo.path(), "unknown").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn service_resolve_truncates_body() {
        let repo = tempfile::tempdir().unwrap();
        let body = "x".repeat(MAX_SKILL_BODY_BYTES + 4096);
        write(
            repo.path(),
            ".agents/skills/big/SKILL.md",
            &format!("---\nname: big\n---\n{body}"),
        );
        let s = service().await;
        let resolved = s.resolve(repo.path(), "big").await.unwrap().unwrap();
        assert_eq!(resolved.content.len(), MAX_SKILL_BODY_BYTES);
    }

    #[test]
    fn home_dir_resolves_from_env() {
        // 环境变量在本进程一定存在其一（Windows CI 与本机均如此）；只验证可解析。
        assert!(home_dir().is_some());
    }
}
