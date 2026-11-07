# 配置预设分层规范

- 日期：2026-11-07（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：P2 项「预设」：命名会话能力配置包（模型/effort/审批策略），解析为显式参数>预设>宿主缺省三层；CLI/JSON-RPC/ACP/桌面四端接入。

## 背景与范围

P2 阶段验收项中的「预设」。此前会话能力全部由调用方逐参数指定：模型与 effort 为每回合参数（`Runtime::run_session_with_attachments` / 桌面 `session_send` / RPC `session.send`），审批策略在宿主侧（CLI `approval::resolve_policy` 三档 ask/auto/never、ACP per-session Policy、桌面固定交互式）。重复性任务组合（例如「glm-5.3 + high effort + auto 审批」跑批量重构、「逐条 ask 审批」做敏感修改）每次都要手动改多处。

本规范定义**配置预设**：一个命名的会话能力配置包，一次选定、三端生效。

**预设包含的能力字段**：`modelId`、`effort`、`approval`（审批策略）。

**「分层」的语义**（解析优先级，各端一致）：

1. 显式参数（CLI flag / RPC params / 桌面级联手选）；
2. 所选预设的字段；
3. 宿主既有缺省回落（`cli.default_model`、非 TTY 必须显式 `--approval` 的 TTY 规则、effort 缺省跟随模型默认）。

**目录来源两层**（与[模型设置规范](model-settings.md)的内置目录/个人覆盖同构）：合成内置预设 `builtin:default`（无任何覆盖、选中即现状行为、只读不落库、save/remove 拒绝），用户自定义预设存 SQLite `settings` 表 `presets` 键（JSON 数组，`Store::get/set` 复用，无 schema 迁移）。

不在范围：附加工作区指令注入与 skills 启停（需动 `context.snapshot` 事件结构，另行立项）；`agent_spawn`/`schedule_write` 接受 presetId；桌面端审批策略接管（桌面保持交互式审批）；SDK 封装（SDK 尚未包装 session.send）；会话绑定预设落库（sessions 表不加列，预设每次调用显式解析，与会话进行中可换模型语义一致）。

## 设计

### 契约（harness-contracts）

- `PresetConfig { id, name, description: Option<String>, model_id: Option<String>, effort: Option<String>, approval: Option<String> }`，serde camelCase，仿 `ModelConfig`。
- `KNOWN_APPROVALS = ["ask","auto","never"]`，仿 `KNOWN_EFFORTS`。
- `PresetInfo` 列表行：`PresetConfig` 全字段 + 计算字段 `broken: Option<String>`（不落库）——modelId 不在目录、模型/供应商已停用、effort 不在该模型 efforts 内时给出原因，列表可读但不可直接用（仿 deepseek-harness agent-preset roster 的 broken 语义）。
- 校验只查模型目录（`ModelService::list_models`/`list_providers`），不做真实凭据读取与网络。

### 预设服务（harness-runtime，新文件 presets.rs）

`PresetService { store }` 挂 `Runtime.presets`（与 `pub models: ModelService` 并列），settings 键 `presets`：

- `list() -> Vec<PresetInfo>`：内置 `builtin:default` 置顶（name「默认（无覆盖）」，无 broken），其后用户预设按存储顺序，逐条计算 broken。
- `save(PresetConfig)`：upsert 校验——id 非空且不等于 `builtin:default`、modelId 必须存在于目录、effort 必须属于该模型 efforts（`KNOWN_EFFORTS` 交集校验交给模型目录）、approval ∈ `KNOWN_APPROVALS`；name 缺省取 id。
- `remove(id)`：builtin 拒绝（`PRESET_BUILTIN`）；不存在时幂等（仿 models remove retain 语义）。
- `resolve(preset_id) -> ResolvedPreset { model_id: Option, effort: Option, approval: Option }`：未知 id 报 `PRESET_NOT_FOUND` 并列可用项；builtin:default 返回全空。

### 各端接入（分层解析）

**CLI run/chat**：`Args` 增 `--preset <id>`。解析链：

- approval：`--approval > preset.approval > resolve_policy 的 TTY 规则`（预设给 ask 而非 TTY 时同样报 `APPROVAL_TTY`）；
- model：`--model > preset.model_id > cli.default_model`（`resolve_model_id` 复用）；
- effort：`--effort > preset.effort > None`。

chat 初始解析同 run；`/approval`、`/model` 斜杠命令仍可运行中覆盖，覆盖后本会话沿用显式值（不回读预设）。

**JSON-RPC（harness-cli/src/lib.rs dispatch）**：新增 `preset.list` / `preset.save` / `preset.remove`；`session.send` 增可选 `presetId`、`modelId` 改为可选，解析 `modelId > preset.model_id > cli.default_model`，均缺报 `MODEL_NOT_FOUND` 列出可用模型（SDK 未包装 session.send，无破坏面）。

**ACP（harness-cli/src/acp.rs）**：`AcpOptions` 增 `preset`（`harness acp --preset`）。`run_turn` 模型解析 `--model > preset.model_id > cli.default_model`，effort 取 preset.effort；`session/new` 初始 Policy = preset.approval（映射三档）或现状 Ask；`session/set_mode` 仍可切换。

**CLI 管理（harness-cli/src/manage.rs）**：`harness presets list|save|remove|default`（仿 ModelsCmd 模式）；`default <id|--clear>` 写 settings 键 `cli.default_preset`。run/chat/ACP 未显式 `--preset` 时读取该键作为第三层回落（与 `cli.default_model` 同层，使默认预设真实生效）；JSON-RPC `session.send` 为编程接口，预设仅随显式 `presetId` 参数生效，不读取该键。

**桌面（src-tauri + frontend/）**：

- 命令 `preset_list/preset_save/preset_remove`（specta 模式同 `default_selection_get/save`），重新导出 bindings。
- `PresetsPanel`（Panel 联合类型加 `'presets'`，仿 ModelPanel）：列表（builtin 只读行、broken 标记）、新建/编辑表单（名称/描述/模型/effort/审批策略）、删除；审批字段标注「仅 CLI/JSON-RPC/ACP 生效」。
- 发送框增预设下拉（「不使用预设」+ 各预设）：选中即以预设 modelId/effort 填充既有三级联（级联持久化沿用 `app.default_selection`，不新增桌面默认键）；手改级联即视为放弃预设。桌面审批保持交互式（安全模型不变）。

## 边界与风险

- 预设不落会话：会话进行中换预设/模型均合法，与现状每回合传参语义一致。
- `session.send` 的 `modelId` 由必填改为可选是契约放宽：SDK 未包装该方法，桌面不经此路径，破坏面为零。
- 预设校验在保存时做，模型后续被删/停用不阻止列表展示，以 `broken` 标记呈现（resolve 时调用方 `build_model_with` 的既有错误自然兜底）。
- 桌面不读 `cli.default_preset`；两端默认互不干扰。
