# 会话用量环形指示器与上下文窗口

- 日期：2026-11-08（计划批准）
- 范围：桌面端会话输入区新增用量环形指示器；模型目录补充上下文窗口元数据；ChatGPT 订阅用量查询接入。
- 参考：zcode Composer 的用量环形与双段悬停面板；codex `last_token_usage` 口径与 `/wham/usage` 端点。

## 交互形态

1. 会话输入区 `composer-bottom` 行内、模型选择器右侧放置独立的环形按钮（15px SVG 圆环，12 点方向起）。
2. 环形本体显示**当前会话上下文占用**（used / 上下文窗口）。颜色按占用：`<70%` 灰绿（默认）、`70–90%` 琥珀、`≥90%` 红。
3. 悬停或点击弹出明细卡（宽 280px，从上方弹出），分两段：
   - **上下文窗口**：`已用 / 窗口` 缩写（62.1k / 200k）+ 占用百分比 + 单条进度条；
   - **套餐额度**：供应商套餐名 + 剩余额度卡（线性条 + 重置时间），复用 `ProviderUsageSnapshot`。
4. 按量计费的供应商（DeepSeek、自定义供应商）无额度段，仅显示上下文段。
5. 模型无上下文窗口值时不渲染环形；自定义模型可在模型编辑弹窗填写「上下文窗口 Token（可选）」。

## 数据口径

- **当前上下文** = 最近一次服务端报告的输入+输出 token 之和（与 codex `last_token_usage.total_tokens` 同口径：Responses 的 `input_tokens` 即整个 prompt）。
- 三协议 usage 字段探测归一化（原样 JSON 已随 `assistant.message` 事件落库）：
  - ChatCompletions：`prompt_tokens + completion_tokens`；
  - Responses：`input_tokens + output_tokens`（input 已含 cached）；
  - Anthropic：`input_tokens + cache_read_input_tokens + cache_creation_input_tokens + output_tokens`。
- **压缩重置**：`compaction.summary` 之前的历史已被清空，其 usage 全部作废；压缩后尚无新请求时显示 0。
- 流式进行中显示上一轮 usage 值；最新 durable 事件到达即自动更新。

## 上下文窗口元数据

- `BuiltinModel` 与 `ModelConfig` 新增 `context_window: Option<u32>`（serde default，旧存储兼容）。
- 内置目录预填上下文窗口值（集中一处便于校正）：DeepSeek 系 128k、GLM Coding 系 glm-5.2/5.3/5.3-flash 为 1M、glm-5-turbo 为 200k；ChatGPT Codex 系跟随 Codex 目录的默认 `context_window`（当前 gpt-6.1-terra 为 500k，其余内置模型为 272k）。Codex 的 `max_context_window` 是配置覆盖上限，不作为用量环形分母。
- 自定义模型缺省为 None → 不显示环形。

## ChatGPT 订阅用量

- `fetch_usage` 供应商分发扩展：ChatGptOAuth 内置供应商走 `GET https://chatgpt.com/backend-api/wham/usage`（`Authorization: Bearer <access token>` + `chatgpt-account-id` 头，凭据复用 `oauth_material`，自动刷新 token）。
- 响应映射：`plan_type` → 套餐名（首字母大写带 ChatGPT 前缀）；`rate_limit.primary_window` → 5 小时卡、`secondary_window` → 每周卡；`remaining = 100 - used_percent`；`reset_at`（unix 秒）缺省回退 `now + reset_after_seconds`，统一折毫秒。
- 复用既有 `ProviderUsageSnapshot` 契约，快照即查即返不落库；模型设置弹窗的额度卡条件同步放宽到 ChatGPT 供应商。

## 非目标

- 上下文分段明细（按系统提示/消息/工具拆分）：需后端扩展采集，留待后续。
- 自动压缩阈值联动：本规范只做展示，不改变 `session_compact` 手动语义。
