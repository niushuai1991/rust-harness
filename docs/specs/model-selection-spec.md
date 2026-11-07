# 模型选择方式优化规范（参考 zcode）

- 日期：2026-11-07（计划拟制）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：CLI `/model`、`/effort` 内联建议面板与切换语义；桌面发送框浮层卡片选择器（搜索、供应商分组、档位滑块、管理模型入口）；默认档位=最高档语义；切换静默持久化与会话流临时切换标记；`/model` 手误不再退出 REPL；`session_compact` 传递档位；`MODEL_NOT_FOUND` 错误列出可用模型。

## 背景与痛点

改造前两端模型选择均有明显摩擦：

- CLI：`/model` 无参数仅 stderr 平铺列表，切换需手打完整长 id（如 `builtin:bigmodel:glm-5.3`）；`/model <错id>` 或切到不支持当前档位的模型会因 `?` 冒泡**直接退出 REPL**；切换仅会话级、不持久化；参数无补全。
- 桌面：发送框为 4 个原生 `<select>` 级联（预设/供应商/模型/档位），模型下拉只列当前供应商条目，无搜索、无跨供应商总览；桌面 `/model` 只是打开重型设置面板；`chooseModel` 将档位重置为模型 `effort` 字段（可为空），与 CLI「保留档位」语义不一致。
- 两端都没有对「切换模型」这一动作的会话内反馈。

参考 zcode 的交互（TUI 内联建议面板 + `/effort` 两步；桌面单按钮分组下拉；`defaultLevel = values.at(-1)`）做对齐。

## 设计

### 档位语义（两端一致）

1. **去掉「默认」伪档位：默认档位即该模型档位列表的最高档**（末位，对齐 zcode）。无档位声明的模型不发送档位参数。
2. 切换模型时：当前档位被新模型支持则保留，否则回落新模型最高档；不再因档位冲突报错退出。
3. 显式传入（CLI `-e`、`/effort <level>`）不支持的档位仍报错（列出可用档位）；KV/预设中的失效档位静默归一化到最高档。

### CLI（harness-cli）

- `line_editor` 弹窗泛化：候选从 `&'static SlashSpec` 扩展为 `Candidate::{Slash,Model,Effort}`；首 token 恰为 `/model`、`/effort` 时展示参数面板（大小写不敏感子串过滤 id/名称/供应商），其余行为（命令补全、Esc 驳回、↑↓ 环绕、绝对行号重绘）不变。面板目录由 chat 每轮 `list_models`/`list_providers` 刷新传入（不缓存，目录读取失败退化为纯命令补全；本地 SQLite 读取，开销可忽略）。渲染：`› ` 选中符 + 当前标记 + 模型行供应商右对齐列。
- 键位：输入即过滤；↑↓ 选择；Enter 将缓冲改写为完整命令（`/model <id>`、`/effort <level>`）并提交；Tab 补全并收起面板；Esc 关闭。
- 新增 `/effort [level]` 命令（注册表、解析、处理）；`/model` 无参列表增加档位列。
- 切换失败（id 不存在、模型停用、构建失败）只打印错误并继续会话；错误列出可用模型 id。
- 切换成功打印临时分隔行 `── 模型已切换：X → Y（档位：Z）──` / `── 档位已切换：X → Y ──`，并**静默**双写持久化默认 `cli.default_model` + `cli.default_effort`（新 KV，失败仅警告）。
- 启动档位解析链：`-e` > 预设 > `cli.default_effort` > 模型最高档（`manage::fallback_effort`）；`run` 与 `chat` 共用。

### 桌面（frontend + src-tauri）

- 新组件 `ModelSelect`：单触发按钮（当前模型名 + 档位徽标）+ 浮层卡片——搜索框、按启用供应商分组（内置/自定义标记）的模型列表（`✓` 当前、停用模型灰显标注、键盘 ↑↓/Enter/Esc）、底部「推理档位」滑块区（居中当前档位 + 重置为最高档 + 刻度圆点轨道）、「管理模型」页脚入口（进 ModelPanel）。选模型后卡片保持展开供连续调档；点击当前模型行或 Esc/外部关闭。
- 发送框改为「能力预设 select（不动）+ ModelSelect」；移除供应商/模型/档位三个原生 select。
- 桌面 `/model` 改为打开浮层卡片（ModelPanel 仍从侧栏与卡片页脚进入，入口未减少）。
- 切换模型后在会话流插入**临时**分隔标记 `.model-note`（React 内存态，不作为会话事件落库，刷新即逝）；不做「已保存」类提示。
- `session_compact` 增加 `effort` 参数，改用 `build_model_with`，压缩模型与用户当前档位一致；`frontend/bindings.ts` 经 `export_bindings` 再生。

### 非目标

- 统一 CLI 与桌面的默认选择 KV（`cli.default_model`/`cli.default_effort` 与 `app.default_selection` 各自保留，与 zcode CLI/桌面分离存储一致）。
- 会话流切换标记落库（zcode 的 `model_change` timeline 事件）；如需回看持久化再立项。
- 最近使用/常用模型置顶；`provider/model$effort` 直选语法；桌面 `/effort` 斜杠命令。
