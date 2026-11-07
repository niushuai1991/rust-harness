# 双供应商、多协议接入规范

- 日期：2026-11-04（计划批准，原文依据）
- 状态：现行规范；能力与验证状态见[能力状态矩阵](../capability-matrix.md)
- 范围：DeepSeek 与智谱 Coding Plan 两个内置供应商的固定目录、三协议适配、配置迁移及桌面和 CLI 接入。

## 目标与接口约定

DeepSeek 与智谱 Coding Plan 两个内置供应商，每个内部选择 Chat Completions、Responses 或 Anthropic。内置名称和端点不可修改，自定义供应商独立管理，不复制密钥。

`ApiMode` 使用 `chatCompletions`、`responses`、`anthropic`；供应商持有多个协议端点，用户配置持有凭据引用和代理，模型引用供应商、协议与模型 ID。旧模型逐项迁移为自定义供应商，不按地址合并。

保留 `ModelConfig` 旧的有效地址等输出字段以兼容 SDK；运行时仅以供应商服务解析的配置发起请求。

审查修订采用显式供应商兼容配置 `compatibility`，复制时保留该属性；跨协议正文按标准简单输入消息回放，Anthropic 关闭全部内容块后才能成功结算。

## 关系说明

- 本规范为模型接入基础约定；[模型设置规范](model-settings.md)在其之上替代"模型标识手动录入"的单层模型形态，内置目录只读、个人覆盖可写。
- 后续打包遵循[总体规范](overall.md)的单安装包策略，默认系统已有 WebView2。
