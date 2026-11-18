# 回合回复呈现：过程折叠与轮尾复制

- 日期：2026-11-19（计划拟制）
- 修订：2026-11-19，补充 Shell 命令折叠展示及历史结果回退规则。
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：桌面端会话时间线的回合呈现形态——一个回合（turn）对用户是一个回复单元：完成回合的过程区默认折叠，复制入口收敛到轮尾总结一处，复制内容为整轮文本合并。纯前端呈现约定，不改后端事件契约。相关：[长会话时间线](long-session-timeline.md)。

## 背景与决策

会话时间线以 `SessionEvent` 平铺存储，`turn.start` / `turn.end` 划定回合边界；前端已按回合分组渲染：完成回合由 `CompletedTurn` 把回合内最后一条 `assistant.message` 独立渲染为总结，其余过程装进 `<details class="turn-details" open>`。消息复制按钮逐条挂在每条用户与助手消息上，且过程区默认展开。

两个呈现问题：

1. 一轮助手回复通常包含多条消息（模型每次工具调用迭代各落一条 `assistant.message`），每条都带复制按钮，复制口径碎片化——复制任意一段都得不到完整回复，且与过程内容的视觉权重相同。
2. 完成回合的过程区默认展开，长回合下思考、工具调用与中间说明把最终总结挤离视窗。

用户拍板对齐目标形态：**一轮对用户是一个回复**——中间过程默认收纳、复制入口只在轮尾总结一处：

1. 完成回合的过程区默认折叠，summary 仍为"本轮已完成，用时X分X秒"，点击展开；
2. 轮尾总结保留常显复制按钮（位置与样式沿用现状），复制内容 = 回合内全部 `assistant.message` 正文合并；
3. 折叠区内的中间助手消息不渲染复制按钮；
4. 用户消息的复制按钮保留；
5. 异常终态回合（`turn.end` 为 error/cancelled）与回合外散事件保持平铺与逐条按钮的现状（诊断形态，便于分段复制现场）。

## 呈现形态矩阵

| 回合形态 | 过程区 | 复制按钮 |
|---|---|---|
| 运行中（回合未闭合） | 平铺展开（现状不变） | 无（流式卡本无按钮） |
| 完成（`turn.end` completed） | 默认折叠，点击 summary 展开 | 仅轮尾总结，复制整轮合并文本 |
| 异常终态（error / cancelled） | 平铺（现状不变） | 逐条（现状不变） |
| 回合外散事件 | 平铺（现状不变） | 逐条（现状不变） |
| 用户消息 | — | 保留常显按钮，复制用户原文 |

运行中回合的 `turn.end(completed)` durable 事件到达后即从平铺切为默认折叠——"运行中展开、结束收起"，与折叠语义一致，无需额外过渡处理。

## 折叠与复制约定

### Shell 命令展示

- 同一时间线分组内按调用 ID 关联 Shell 调用与结果；匹配唯一且顺序正确时合为一张卡片，缺失、重复或未匹配时保留独立事件。其他事件和文件修改分组的顺序保持不变。
- 命令卡片默认折叠，摘要展示单行命令及实际状态。展开后显示原始命令；超过三行可展开完整命令，长命令和输出限高滚动。
- 解析已有 Shell 输出中的标准输出、标准错误和退出码，空输出明确标注；无法解析时原样展示，并保留截断提示和附件下载入口。无结果显示等待结果，不虚构耗时和实时输出。
- 已完成回合的过程区仍按本规范默认折叠，展开过程后命令卡片仍默认折叠。

### 折叠边界

- 折叠范围为回合内轮尾总结之前的全部过程事件：中间 `assistant.message` / `assistant.attempt`、`tool.call` / `tool.result`、reasoning 等；`turn.start`、`turn.end`、`user.message` 不入折叠区（用户消息与轮尾总结常驻）。
- 轮尾总结取回合内**最后一条** `assistant.message`（现有 `CompletedTurn` 的 `lastIndexOf` 语义不变）。
- 折叠区内的中间助手消息仍按完整消息卡渲染（头像、正文、reasoning 折叠块），仅去掉复制按钮。
- 每条助手消息内的 reasoning 折叠块、工具卡折叠行为均不变。

### 复制内容合并

- 合并范围：回合内全部 `assistant.message` 的 `data.content`（经 `printable` 处理），按 `\n\n` 连接；
- 不包含：`reasoning`（思考过程）、工具参数与输出、`assistant.attempt`（失败半截内容）；
- 回合内仅一条 `assistant.message` 时，合并结果即该条正文，行为与现状一致；
- 回合内没有 `assistant.message`（如纯工具异常回合）时无总结卡，无复制入口。

## 实现要点

- `EventCard` 增加可选 props：`copyText?: string`（覆盖该条消息的复制内容）与 `hideCopy?: boolean`（隐藏复制按钮）；assistant 分支复制内容取 `copyText ?? printable(data.content)`；user.message 分支不动。
- `EventList` 增加可选 `hideCopy` 透传给内部 `EventCard`；`FileEditGroup` 本无复制按钮，不受影响。
- `CompletedTurn`：`<details class="turn-details">` 去掉 `open` 属性；过程区以 `hideCopy` 渲染；轮尾总结以 `copyText=整轮合并文本` 渲染。
- 纯前端改动：不新增事件 kind，不改 `SessionEvent` / `TimelinePage` / `LiveEvent` 契约，不动后端投影与存储。

## 测试与验收

- 遵循 TDD：先写失败测试，确认失败原因正确后完成最小实现。
- 完成回合场景（mock timeline 含 turn.start → user → 中间 assistant → 工具调用 → final assistant → turn.end(completed)）：
  - "复制消息"按钮全场仅一个（轮尾总结），折叠区内中间助手消息无按钮；
  - 点击轮尾复制按钮，剪贴板收到整轮合并文本；
  - 过程区默认折叠、点击 summary 后展开（jsdom 对未展开 `<details>` 的可见性语义以实测为准，兜底断言 `details.open` 布尔值）。
- 用户消息按钮、单条散事件按钮等既有用例保持通过。
- 命令：`npm test` 全量（仓库根运行）、`npm run build`；Rust 四命令按提交约定照跑，确认纯前端改动无影响。
