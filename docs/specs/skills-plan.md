# Skills（技能）支持计划

计划批准日期：2026-11-07。

## 背景与范围

当前对技能只有雏形支持：`workspace_context`（`crates/harness-runtime/src/agent.rs`）扫描工作副本内 `SKILL.md` 文件，在系统提示中列一行裸路径「可按需读取的技能：…」。模型不知道每个技能的用途，无法判断何时使用；无用户级技能、无启停管理、无显式触发。配置预设计划（2026-11-07）明确将「skills 启停」列为另行立项项，本计划即该立项；同时落地斜杠指令计划当时列为非目标的 codex `$` skill 体系。

本计划交付：

- **技能目录与发现**：项目级 `.agents/skills/<name>/SKILL.md` + 用户级 `~/.agents/skills/` + 工作副本内其余任意 `SKILL.md` 兜底扫描（向后兼容 `scripts/p1-desktop-cdp.mjs` fixture 的 `.skills/p1/SKILL.md`）。
- **catalog 注入**：系统提示注入带 name/description 的技能目录与触发规则文本，模型自主按描述匹配并按需读取。
- **`$name` 显式提及**：用户消息中 `$name` 命中技能时宿主读取 SKILL.md 全文注入该回合；CLI 行编辑器与桌面输入框提供 `$` 补全弹窗。
- **启停管理**：技能可禁用/启用，禁用名单落 SQLite；CLI 管理子命令 + REPL 查看命令 + 桌面管理面板。

设计参考 codex skills 体系（SKILL.md + frontmatter + catalog 注入 + `$` sigil + 宿主全文注入），按本项目体量简化。参考仓库：`D:\code\codex`（`codex-rs/skills`、`codex-rs/ext/skills`）。

不在本次范围（边界）：技能参数占位符（`$ARGUMENTS`）、插件命名空间技能、系统内置技能、Claude commands 迁移管道、token 级预算（仅字符截断）、CLI console 与 ACP 对 `skill.loaded` 事件的显示/透传、`skills.list`/`skills.read` 类专用工具（读取走现有免审批 `read`）。

## 设计

### 技能模型（harness-runtime，新文件 skills.rs）

- `SkillInfo { name, description: Option<String>, path, scope: "project"|"user", enabled, broken_reason: Option<String> }`，serde camelCase（桌面 bindings 消费）。
- **目录与发现**（纯函数）：
  - 项目级：工作副本内 `.agents/skills/*/SKILL.md`，scope=`project`；
  - 兜底：工作副本内其余任意 `SKILL.md`（复用 `harness_platform::list_files` 相对路径扫描，遵循 gitignore，上限 100 个），scope=`project`；
  - 用户级：`~/.agents/skills/` 递归扫描（`USERPROFILE`/`HOME` 解析，缺失跳过不报错），scope=`user`；
  - 排序：project 在前、user 在后，同级按 name 排序；同名时项目级优先。
- **frontmatter 解析**（手写，不新增 yaml 依赖）：`---` 围栏内逐行 `key: value`，只认 `name`/`description`，容忍未知字段（第三方技能字段不报错）。`name` 缺省取父目录名，限 `[A-Za-z0-9_-]` 且 ≤64 字符，非法时 `broken_reason`；`description` 缺失不判 broken，目录显示「（无描述）」（fail-open）；SKILL.md 读取失败时 `broken_reason` 记录原因，不阻塞其余技能。

### catalog 注入与 workspace_context

- `workspace_context` 签名改为接收现成 catalog 文本（保持纯函数可测）；调用点（`steps()` 每回合开头）先经服务计算再传入。**不改 `context.snapshot` 事件结构**（`data.content` 仍为系统提示全文），化解配置预设计划点名的顾虑。
- catalog 格式（无启用技能时整段不注入）：

  ```
  ## 可用技能
  - {name}: {description} ({path})
  使用规则：用户消息以 $name 提及某技能、或任务与上述描述明确匹配时，读取对应 SKILL.md 并遵循其内容执行。技能只在被提及或匹配的回合使用。
  ```

- 截断：description 截 200 字符；catalog 总量超 8000 字符截断并注明「（其余技能省略）」。每回合重算，启停与新增技能下一回合自然生效。

### `$name` 显式提及（runtime 层统一解析，全端受益）

- 挂点：回合入口（append `user.message` 处）。扫描 `\$([A-Za-z0-9_-]+)`，仅命中启用技能名单的名字才算提及（`$PATH` 等环境变量因不在名单天然排除）；原文不改写（同 codex）。
- 命中后 append 新事件 kind=`skill.loaded`，data `{name, path, content}`（SKILL.md 剥离 frontmatter 后的正文，32KiB 字节截断，风格同 AGENTS.md 的 32000 字符；name/description 已在 catalog 与 `<skill>` 标记中，正文不重复携带元数据）；同回合多个提及逐个注入。
- `project_messages` 新增 `skill.loaded` 分支：转为附加 user 消息 `<skill name="…" path="…">\n全文\n</skill>`，紧随其 user.message 之后。
- `resolve(root, name)`：按排序取第一同名（简化，不拒绝重名——见风险）。

### 启停管理（仿 presets.rs 存储模式）

- `SkillService { store: Store }` 挂 `Runtime`（与 `pub presets` 并列）；禁用名单存 SQLite `settings` 表单键 `skills.disabled`（JSON 字符串数组，元素为技能 path），`Store::get/set` 复用，无 schema 迁移；新技能默认启用。
- 方法：`list(root) -> Vec<SkillInfo>`（扫描 + 禁用标记）、`catalog(root) -> String`、`resolve(root, name) -> Option<SkillContent>`、`set_enabled(path, enabled)`。

### 各端接入

**CLI 管理（manage.rs）**：`harness skills list|enable|disable`（仿 presets 子命令模式）；`list` 输出 name/scope/path/description/broken 与启停状态，`enable/disable` 以 path 为参写禁用名单。

**CLI REPL（slash.rs + chat.rs）**：注册 `/skills` 只读命令，按排序列出当前工作副本生效技能（含禁用标记）；不影响审批与回合流程。

**CLI 行编辑器（line_editor.rs）**：`popup_mode` 扩展——光标处 token 以 `$` 开头时返回新 `PopupMode::Skills`，候选来自注入的技能名单（仿 `picker.models` 注入路径）；`apply_completion` 补全为 `$name ` 并补尾随空格；Esc dismiss 与现有 slash 弹窗语义一致。

**桌面（src-tauri + frontend/）**：

- 命令 `skill_list(workspace_id)`（经 workspace root 扫描，settings 叠加启停）、`skill_set_enabled(workspace_id, path, enabled)`（specta 模式同 `preset_list/preset_save`），重新导出 bindings。
- `frontend/types.ts` 手写镜像 `SkillInfo`；新增 `SkillsPanel`（仿 PresetsPanel 双栏：列表 + 启用开关 + broken 标记 + scope 徽标），Panel 联合类型加 `'skills'`。
- composer `$` 补全：镜像现有 slash-menu 模式，输入 `$` 弹技能候选（来自 `skill_list`），选中补全 `$name `。
- `EventCard` 加 `skill.loaded` 分支：一行「已加载技能 {name}」+ 可展开全文；不改 `context.snapshot` 渲染（维持不显示）。

## 边界与风险

- 重名从简：`$name` 与 catalog 引用均按排序取第一（项目级优先），不采用 codex 的「重名拒绝纯名字匹配」；个人项目技能数量少，风险可接受。
- `$` 解析按「命中启用技能名单才算提及」白名单式防误报，但仍可能把恰与技能同名的普通文本解析为提及；后果是注入一段技能全文，不丢用户语义。
- 用户级目录依赖 `USERPROFILE`/`HOME` 环境变量，缺失时静默跳过用户级。
- `skill.loaded` 为持久事件：会话重放时注入内容随历史重现，与 `user.message` 同生命周期；技能改名/删除后旧会话重放不受影响（事件已含全文）。
- `scripts/p1-desktop-cdp.mjs` 断言 system 消息含 `SKILL.md` 字样：catalog 行含路径，预期仍通过，实施时运行确认。

## 验证

- TDD 先红后绿：runtime skills 单测（frontmatter 解析矩阵、三源扫描与排序、禁用标记、catalog 渲染与截断、resolve 命中/重名/禁用）；`agent_flow.rs` 集成测试（catalog 注入 system、`$name` 触发 `skill.loaded` 与消息注入、禁用后不注入）；CLI manage/slash/line_editor 各带测试；前端 Vitest（SkillsPanel 列表与启停、composer `$` 补全、EventCard skill.loaded、bindings 导出）。
- 标准四连：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`；`npm ci && npm test && npm run build`（worktree 根目录）。
- 真实模型端到端（catalog 出现与模型自主读技能、`$name` 注入回合）与桌面 UI 抽查留给用户。
