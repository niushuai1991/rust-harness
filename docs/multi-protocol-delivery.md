# 0.1.2 双供应商、多协议交付

内置供应商只有 **DeepSeek** 和 **智谱 Coding Plan**，每个内部选择 Chat Completions、Responses 或 Anthropic Messages。内置名称和协议端点固定；需要不同地址时新增自定义供应商，或复制内置供应商后编辑。

## 使用

1. 打开“模型设置”，选择供应商，填写 API Key 和可选代理。
2. 在该供应商内部选择协议，检查只读的有效 API 地址，填写模型 ID 和可选输出 Token 上限。
3. 保存模型后，可运行“测试已保存配置”。测试会发送少量实际生成请求并消耗额度。
4. 同一供应商内的模型共用凭据和代理；复制供应商保留协议端点映射、兼容规则及当前协议/模型/输出上限，使用新的模型标识和独立凭据引用，密钥不会复制。
5. 自定义供应商可编辑各协议端点。删除供应商前先删除其关联模型；不能删除正在被模型引用的协议端点。

默认模型为 DeepSeek 的 deepseek-flash、智谱的 glm-5.3；用户可填写其他账号可用的模型 ID。预置供应商不会自动调用模型服务。

## 实现与兼容

- 三个协议适配器共用统一历史和 SSE 分帧，分别处理认证、工具请求与结果、推理、用量和结束状态。
- Responses 使用完整历史回放，不依赖服务端会话 ID；加密推理项和 Anthropic 签名内容按同一供应商、模型、协议及端点回放，跨路由仅转换通用内容。
- 请求快照记录实际协议请求和路由，不记录认证头。错误响应仅有限读取并使用白名单错误分类，服务端文字不会进入日志。
- 桌面、CLI、后台定时任务和模型连接测试使用同一配置服务；上下文压缩剔除私有协议回放信息。
- SQLite 格式由 1 升到 2。升级前使用 SQLite 在线备份（包括 WAL 已提交页），首次备份为数据库旁的 .db.v1.bak，重试保留旧备份并产生独立新备份。
- 原模型逐项迁移为自定义供应商，保留地址、模型、凭据引用及代理，不按地址自动合并为内置供应商。旧版本会拒绝打开格式 2 数据，不能直接降级使用新数据库。

## 验证

- 本地真实 HTTP/SSE 完成三协议“并行工具请求→写入审批→工具结果→继续回答→重启继续会话”；未调用真实付费供应商。
- 六条固定路由、分块 UTF-8、多行 SSE、截断、错误、取消、签名与加密推理回放、跨协议正文、Anthropic 未关闭内容块、密钥回显脱敏均有回归。
- 供应商保护、复制兼容配置、并发保存、引用端点保护、迁移失败回滚及重试、未来版本拒绝均有回归。
- 独立审查发现的复制兼容参数、未关闭 Anthropic 块和 Responses 正文输入形状已修复；定时任务测试隔离了手动 tick 与后台 tick 的竞争。

最终测试及安装包结果见[交付记录](delivery.md)。真实供应商账号授权、签名发布，以及干净 Windows 上安装/卸载和无 WebView2 场景未在本次验证。

## 协议依据

- [OpenAI Responses 迁移说明](https://developers.openai.com/api/docs/guides/migrate-to-responses)
- [OpenAI SDK 简单输入消息定义](https://github.com/openai/openai-python/blob/main/src/openai/types/responses/easy_input_message_param.py)
- [DeepSeek Responses 兼容说明](https://api-docs.deepseek.com/guides/responses_api/)
- [Anthropic 流式消息说明](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [智谱 Coding Plan 接入说明](https://docs.bigmodel.cn/cn/coding-plan/quick-start)

