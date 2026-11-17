# 全局指令文件（数据目录 AGENTS.md）规范

- 日期：2026-11-17
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 修订记录：2026-11-17 全局文件改用普通文件读取，允许符号链接；不再复用副本工作区的受控 `read_file`。
- 范围：数据目录下全局指令文件 `AGENTS.md` 的读取、静默跳过、截断与系统提示注入；CLI/桌面端/ACP/子代理共用同一注入链路。

## 背景与范围

项目此前只注入工作区级指令文件（执行根下的 `AGENTS.md`/`CLAUDE.md`，见 `workspace_context`），用户在数据目录（默认 `~/.RustHarness`，见 `harness-platform::paths::resolve_data_dir`）放置的个人全局指令不会被会话感知。本规范约定数据目录级全局指令文件的自动读取与注入，使所有会话（含子代理）默认获得跨项目的个人指令。

不在范围：override 优先文件（如 `AGENTS.override.md`）、多级祖先目录发现、配置项控制开关、CLI 管理命令。以上留作后续扩展。

## 设计约定

- **路径**：`resolve_data_dir(...)` 解析出的数据目录下的 `AGENTS.md`（显式参数 > `RUST_HARNESS_DATA_DIR` 环境变量 > 默认主目录点目录）。读取使用普通文件读取（如 `tokio::fs::read_to_string`），**允许符号链接**：全局文件是用户自有配置，可能指向统一维护的指令文件；副本工作区的受控读取会拒绝 reparse point，不适用于此处。
- **发现规则**：单文件。文件缺失、读取失败或内容 trim 后为空白时静默跳过（与技能用户级目录缺失静默跳过一致），不产生警告或事件。
- **注入位置**：系统提示（`workspace_context` 纯函数），身份段之后、项目级 `AGENTS.md`/`CLAUDE.md` 之前、技能 catalog 之前；全局与项目级同时存在时全部注入，全局在前。段落标题为数据目录下实际路径（`{数据目录}/AGENTS.md:`），与项目级的 `{文件名}:` 标题自然区分。
- **截断**：32000 字符（`chars().take(32000)`），与项目级指令一致。
- **纯函数契约**：`workspace_context` 接收调用点已渲染好的段落文本（同[Skills 规范](skills.md)对 catalog 的约定），文件 IO 只发生在 `steps()` 每回合开头的调用点。
- **事件契约**：不改变 `context.snapshot` 事件结构（`data.content` 仍为系统提示全文），全局段落随每回合重算进入快照；修改全局文件后下一回合自然生效。
- **各端**：注入发生在 runtime 层 `steps()`，CLI、桌面端、ACP 与子代理会话自动生效，无端侧改动。

## 验证

- `crates/harness-runtime/tests/agent_flow.rs`：全局注入且先于项目级、32000 字符截断、空白内容静默跳过、符号链接跟随注入（先红后绿）。
- 交付前按仓库约定执行 `cargo fmt --check`、`cargo clippy`、`cargo test`、`cargo build`（均 `--all-features`）。
