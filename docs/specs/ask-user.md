# ask_user 提问工具（Agent 向用户提问）

- 日期：2026-11-12（计划批准）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：会话中模型向用户提问的 `ask_user` 内置工具：问题+候选项参数契约、挂起等待与自动结算语义、`ask.request`/`ask.resolved` 事件对、CLI chat 交互与 run 非交互结算、JSON-RPC `ask.*`、桌面提问 Modal 与时间线回放卡；ACP 端 v1 以无人在场自动结算，elicitation 转发留待后续。设计参考 zcode AskUserQuestion 模式。

## 背景与决策

会话中模型原本只能把疑问写进回复文本结束回合，没有结构化的「暂停等用户决策再继续」机制。2026-11-12 用户确认三项决策：

1. **桌面端提问交互用浮层 Modal**（与审批卡片同形态：轮询 + Modal + 按钮组），不做时间线内嵌交互表单。
2. **ACP 端留待后续**：elicitation/create 转发不在本规范范围（[ACP 规范](acp.md) 的 `elicitation/*` 非目标维持）；ACP 宿主以无人在场自动结算。
3. **非交互场景自动结算继续跑**：`harness run` 非 TTY stdin 等无人在场环境，提问立即以固定答复结算，不报错不摧毁回合。

## 工具契约

工具名 `ask_user`，内置工具（harness-runtime `builtin_tools()`），单问题：

```json
{"question": "先改哪个文件？", "options": [{"label": "session.rs", "description": "存储层"}, {"label": "auth.rs"}]}
```

- `question` 必填；`options` 可省略（纯自由文本提问），有候选项时用户仍可自由输入。
- 免审批：`ask_user` 在 `requires_approval` 白名单中——提问本身不得再触发审批流（否则死锁）。
- 挂起等待上限 **600 秒**（与审批兜底一致），且 `steps()` 的 180 秒工具执行时限对 `ask_user` 放宽到同一上限；取消（回合 cancel）即视为未答复。
- 工具结果（`tool.result`，`is_error=false`，模型可见）：
  - 选项：`用户选择了「label」（description）`；
  - 自由文本：`用户答复：…`；
  - 无人在场：`当前无用户在场（未答复），请基于现有信息自行决策并继续。`；
  - 超时：`等待答复超时，请基于现有信息自行决策并继续。`

## 事件对与挂起通道

- `ask.request`：data 为 `AskRequest{id, sessionId, question, options}`（harness-contracts，camelCase）。
- `ask.resolved`：data 为 `{id, answer, source}`；`AskSource ∈ {selected, text, unattended, timeout}`。
- 挂起通道与审批同构：runtime `Questions` pending 表（`HashMap<id, (AskRequest, oneshot::Sender<AskAnswer>)>`）+ 宿主回传 `resolve(id, answer)`；答复以 `tool.result` 落事件回灌模型历史（`project_messages` 既有规则），回合不因等待而结束。
- pending 表全局共享：子代理提问按 `AskRequest.session_id` 路由到宿主（同审批），宿主须覆盖作用域内子会话（[CLI 规范](cli.md) 审批作用域约定同样适用）。
- 崩溃恢复不重放提问等待：pending 为内存态，重启后未闭合回合按既有 `Store::recover` 语义收尾。

## 四端行为

| 宿主 | 行为 |
|---|---|
| CLI chat（TTY） | `ask.request` 到达后渲染问题+编号选项（stderr），stdin 行读：序号选选项、其余文本为自由答复、空行重试、EOF 视为无人在场结算 |
| CLI run | 非 TTY stdin 设 `Runtime::set_unattended(true)`：runtime 不经挂起直接落 request+resolved（source=unattended）；TTY run 交互同 chat |
| JSON-RPC | `ask.list` / `ask.resolve`（仿 `approval.*`）；`ask.request` 经 `session.event` 通知推送 |
| 桌面 | `question_list` / `question_resolve` 命令；前端 2 秒轮询（与审批同 effect），弹出「Agent 需要你的输入」Modal（选项单选或自由输入，「无法回答」按 unattended 提交）；等待提问时隐藏思考指示；时间线 `ask.request` 卡片按 id 配对 `ask.resolved` 显示答复/未答复 |
| ACP | v1 不做交互转发：`acp` 子命令构造实例即设 unattended，提问自动结算；`tool.call`/`tool.result` 经既有通用转发展示（标题「提问 …」）；elicitation 转发留待后续专项 |

## 不变量

- `ask_user` 免审批白名单；审批策略五档决策矩阵（[权限模式规范](permission-modes.md)）不感知提问。
- 提问不触碰「副本 + apply」边界与 Windows 沙箱（[总体规范](overall.md) 安全不变量）：它只是回合内的一次宿主输入等待。
- 事件流完整：无论挂起、自动结算还是取消，`ask.request`/`ask.resolved` 均成对落库，回放语义完整。
- 无人答复不摧毁回合：无人在场与超时都以 `is_error=false` 的引导性答复结算，模型自行决策继续。
