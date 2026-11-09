# 每回合 Token 预算

- 日期：2026-11-09（计划批准）
- 范围：共享 Runtime 对每条用户消息累计模型 Token 用量；桌面 UsageRing 展示并编辑持久化上限。
- 参考：Codex rollout budget 的累计口径；默认统计输出与未缓存输入，缓存命中不计。

## 预算范围与配置

- 每条顶层用户消息创建独立预算，默认上限为 1,000,000 token；同一消息创建的所有子代理继承并共用该预算。下一条用户消息重新从 0 开始。
- 上限保存于 settings KV 的 `app.token_budget_limit`，缺省读取 1,000,000；有效范围为 1 至 `Number.MAX_SAFE_INTEGER`。UsageRing 可编辑，上限修改从下一条用户消息起生效。
- 桌面命令 `token_budget_limit_get` / `token_budget_limit_set` 读写上限；桌面、CLI、JSON-RPC 与 ACP 的会话运行均由同一个 Runtime 预算执行逻辑保护。
- 桌面自动标题 `session_title_generate` 是独立的一次性元数据请求，不计入用户回合预算；它不提供工具，且有 30 秒超时，不会启动工具调用循环。

## 用量口径

- 预算范围内每次成功模型响应都累计一次完整的输入与输出用量；累计值为输出 token 加未缓存输入 token，各项权重均为 1。
- Chat Completions：`prompt_tokens - prompt_tokens_details.cached_tokens + completion_tokens`；缓存 token 大于 `prompt_tokens` 时用量无效。
- Responses：`input_tokens - input_tokens_details.cached_tokens + output_tokens`；缓存 token 大于 `input_tokens` 时用量无效。
- Anthropic：`input_tokens + cache_creation_input_tokens + output_tokens`；`cache_read_input_tokens` 是缓存命中，不计入。
- 用量解析发生在模型响应成功后，即使模型返回的工具调用无效也不能绕过累计。HTTP 流在工具参数解析失败前已收到 usage 时，仍先记账再返回协议错误；若只有不完整用量，则以 `TOKEN_USAGE_UNAVAILABLE` 停止当前回合。

## 上限行为

- 每次发出模型请求前检查共享预算。响应 usage 使累计值达到或超过上限后，不再开始后续模型请求，并取消该回合其他活动会话。
- 响应不含工具调用时，正常交付这条最终回答；响应含工具调用时，保留模型响应与工具调用记录，为每项写入“预算耗尽、未执行”的错误结果，然后以 `TOKEN_BUDGET_EXCEEDED` 结束回合。
- Runtime 向根会话及当前子会话写入 `token.budget` 进度事件，包含 `turnId`、`usedTokens`、`limitTokens`、`status`，供 UsageRing 展示。该事件仅用于进度回放，不参与预算计算。
- usage 只能在模型返回后获知；已发出的并发模型请求可能在取消前完成，因此实际累计值可能超过上限。新请求与超限响应的工具副作用仍会被阻止。

## 桌面交互

- UsageRing 增加“本轮 Token 预算”累计值与进度条，以及每回合上限输入和保存操作；设置从下一条用户消息生效。
- 当前模型没有上下文窗口元数据时，仍渲染预算入口，环形显示本轮预算占用；有上下文窗口时，环形保持显示当前上下文占用。
- 缓存与预算不会改变既有上下文窗口统计或 compaction 行为。
