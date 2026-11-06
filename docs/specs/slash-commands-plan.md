# 输入框斜杠快捷指令计划

- 日期：2026-11-06（计划批准）
- 状态：已实施，Rust 125 项与前端 59 项测试通过（2026-11-06）；两端交互抽查待用户。2026-11-07 修订：新增 /plan /goal 命令与桌面端加号添加菜单，详见修订记录。
- 范围：CLI `harness chat` 的 `/` 斜杠命令注册表、crossterm 行编辑与实时补全弹窗、五个新增命令；桌面端 composer 命令菜单与 `session_compact`。设计参考 codex CLI（`tui/src/slash_command.rs`、`bottom_pane/command_popup.rs`）。

## 背景与目标

`chat` REPL 已有 7 个斜杠命令（手写 match，`chat.rs`），但输入 `/` 时无补全提示，且 `cli-plan.md` 将 REPL 体验升级预留为"后续计划"。本计划落地该预留项：输入 `/` 实时弹出命令候选菜单（↑↓ 选择、Tab 补全、Esc 关闭），同时补齐常用命令，并同步给桌面端输入框。

**非目标**：不引入 ratatui 全屏 TUI（维持 cli-plan 决策）；不做自定义命令与 codex `$` skill 体系；不做历史持久化（会话内存历史即可）。

## CLI 设计

### 四层结构（对齐 codex）

1. **注册表**（`crates/harness-cli/src/slash.rs`）：`SlashSpec{name, aliases, description, arg_hint}` 静态表 `SLASH_COMMANDS`，数组顺序即弹窗展示顺序；`find_spec` 按规范名或别名查找；`filter_slash_commands(query)` exact > prefix 过滤（各自保持注册顺序，容忍前导 `/`，空查询返回全量）；`ChatCommand` 枚举 + `parse_command` 查表解析。
2. **行编辑器**（`crates/harness-cli/src/line_editor.rs`）：逻辑状态机 `Editor`（缓冲/光标/历史/弹窗，可单测）+ crossterm 渲染薄层（`map_event`/`redraw`/`read_line`）。
3. **补全弹窗**：输入以 `/` 开头且光标在首 token 内时显示；每行 `› /cmd [hint]  描述`，最多 8 行、随选中项滚动；↑↓ 循环选择。
4. **分发**（`crates/harness-cli/src/chat.rs`）：`parse_command` 结果 match 分发，命令与普通消息共用 `turn_once` 提交入口。

### 交互规格

| 按键 | 弹窗激活 | 弹窗关闭 |
|---|---|---|
| ↑/↓ | 循环移动选中项 | 翻内存历史（保留草稿，↓ 到底恢复） |
| Tab | 补全规范名（`/name `），保留已输参数文本 | 无操作 |
| Enter | 应用选中项补全后提交 | 提交当前行 |
| Esc | 关闭弹窗并记录当前 token；token 变化前不再自动弹出 | 无操作 |
| Ctrl+C | 行非空清空行；行空等同 `/quit` 退出 | 同左 |
| Ctrl+D | 空行时返回 EOF | 同左 |

弹窗自动显隐：光标移出首 token（进入参数区）、缓冲不再是 `/` 开头、候选为空时自动关闭；文本变化实时重过滤。

**raw mode 生命周期**：仅在 `read_line` 等待输入期间启用，提交/Ctrl+C/Ctrl+D 后立即恢复。回合流式输出（console 裸写 stdout/stderr）与审批 y/n（`run::read_confirmation` 的 `read_line`）不受影响，维持 cli-plan 的单一 stdin 串行约定。

**渲染细节**：光标定位按 `unicode-width` 计算显示宽度（CJK 双宽）；弹窗画在输入行下方，每次按键全量重绘，弹窗收起时清除上一帧残留行。

### 命令清单（展示顺序）

| 命令 | 别名 | 描述 | 参数 |
|---|---|---|---|
| /help | /h | 显示可用命令 | - |
| /model | - | 查看或切换模型 | [id] |
| /status | - | 显示会话与运行状态 | - |
| /clear | /new | 重开新会话 | - |
| /compact | - | 压缩历史为摘要 | - |
| /init | - | 在项目根生成 AGENTS.md | - |
| /resume | - | 切换到已有会话 | [id] |
| /sessions | - | 列出会话 | - |
| /diff | - | 查看副本改动 | - |
| /apply | - | 审阅后同步到原项目 | - |
| /approval | - | 查看或切换审批策略 | [auto\|ask\|never] |
| /quit | /exit /q | 退出 chat | - |

### 新增命令语义

- `/clear`：`store.create_session(workspace_id, "新会话")` 后重绑本地 `session_id`；原会话保留，提示可用 `/resume` 切回。runtime 无会话删除 API，语义为"重开"。
- `/compact`：调用 `harness_orchestration::compact_session`（与 JSON-RPC `session.compact` 同路径），拿会话边界锁与回合串行；`console.rs` 补 `compaction.summary` durable 渲染（`[历史已压缩] <摘要>`）。
- `/init`：以预置 `INIT_PROMPT`（chat.rs 常量）走 `turn_once` 发起回合，模型在隔离副本生成 AGENTS.md；完成后提示 `/diff` 查看、`/apply` 审阅同步到原项目。不直接写原项目，遵守副本约束。
- `/status`：本地打印会话 id、工作区 `source_path`/副本 `replica_path`（`runtime.workspaces.get`）、当前模型、审批策略。
- `/resume`：无参列出本工作区最近 10 个非子代理会话（`updated_at` 倒序，标记当前）；带 id 校验属于该工作区后重绑 `session_id`。

`/help` 文案由注册表生成，新增命令无需改帮助文本。

## 桌面端设计

- **注册表**（`frontend/slashCommands.ts`）：桌面端可执行子集镜像 `/clear /compact /init /model /diff /status /help`，同名过滤函数（exact > prefix）。
- **菜单**：`text` 以 `/` 开头且光标在首 token 内时，在 `form.composer` 上方渲染候选菜单（composer 加 `position:relative`）；↑↓ 选择、Tab 补全、Esc 关闭、Enter 执行选中，全部 `preventDefault` 且绕过 IME 组合；点击菜单项同样执行。
- **执行映射**（复用现有动作）：`/clear`→`createSessionFor`；`/model`→打开 models 面板；`/diff`→打开 diff 面板；`/init`→以预置 prompt 走 `send()`（文本与 CLI `INIT_PROMPT` 保持一致，以 CLI 为准）；`/status` `/help`→本地提示信息；`/compact`→新 Tauri 命令。
- **`session_compact`**：`src-tauri/src/commands.rs` 新增（`sessionId` + `modelId`），调用 `harness_orchestration::compact_session`，注册进 `collect_commands!` 并重新导出 specta bindings；前端完成后重拉 `session_events`。

## 验证计划

- CLI 单测（TDD）：注册表顺序/别名/过滤排序、`parse_command` 全量与新命令、编辑器状态机（插入/删除/光标边界/历史草稿/弹窗显隐与 Tab/Esc/Enter 语义）、help 文案覆盖注册表、console compaction 渲染。
- 提交前四连：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`cargo build --all-features`。
- 桌面端：`slashCommands.ts` 过滤纯函数测试；`App.test.tsx` 用 `fireEvent.keyDown` 补键盘/菜单测试（worktree 内先 `npm ci`）。
- 待用户交互抽查：`harness chat` 输 `/` 看弹窗、Tab/Esc/↑↓、`/init`、`/compact`；桌面端菜单点击与键盘导航。

## 风险与边界

- crossterm raw mode 依赖 Windows Terminal/ConPTY 兼容性；弹窗绘制在终端底部最后一行时可能随终端滚动产生错位，属已知边界，编辑异常不影响提交链路。
- 桌面端 bindings 为 specta 生成物，不手改；`/init` prompt 文本两处维护，以 CLI 常量为单一事实源。

## 修订记录

- 2026-11-06（计划批准）：初版。
- 2026-11-07（扩展实施）：新增 `/plan`、`/goal` 斜杠命令与桌面端 composer 加号"添加内容"菜单。两端注册表镜像新增两命令（CLI 展示顺序插于 `/init` 之后），预置提示词 `PLAN_PROMPT` / `GOAL_PROMPT` 以 CLI `chat.rs` 常量为单一事实源；带参数发送时拼接任务/目标文本，无参数打印用法提示。桌面端"添加附件"文本按钮改为 + 号图标按钮，弹出"添加内容"菜单：附件（沿用 `addAttachment` 文件选择导入）、计划（插入 `/plan `）、目标（插入 `/goal `）；发送时由 `expandSlashPrompt` 把首命令展开为预置提示词。执行层复用模型既有 `plan_write` / `goal_write` 工具，事件流仍是事实来源，计划/目标面板保持只读派生视图，不新增 UI 直写通道。交互形态参考 codex `/plan`（计划模式）与 `/goal`（持久目标）、zcode 输入框加号菜单（插入斜杠命令）。
