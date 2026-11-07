# 模型设置：ChatGPT 账号 OAuth 登录计划

- 文档日期：2026-11-07（计划批准）
- 状态：实施中
- 范围：模型设置新增"ChatGPT 账号"内置供应商（OAuth 登录，非 API key）；PKCE 授权码流程 + 本地回环回调 + token 刷新与吊销；ChatGPT 后端 Responses 协议适配；桌面端登录 UI 与 CLI `login`/`logout` 子命令。两端共用同一数据目录与凭据存储，一处登录两端可用。

## 背景与目标

模型设置目前只支持静态 API key 认证（Windows 凭据管理器存储），无法使用 ChatGPT 订阅（Plus/Pro）背后的 Codex 模型访问能力。参考 codex CLI（`D:\code\codex`，用户指定参考仓库）的成熟实现，新增"Sign in with ChatGPT"：

1. 模型设置出现第三个内置供应商 `chatgpt`，认证方式为 OAuth 登录而非 API key。
2. 登录走浏览器 OAuth（PKCE S256 + 本地回环回调），成功后保存 token 并支持自动刷新、登出吊销。
3. 该供应商的模型经 `https://chatgpt.com/backend-api/codex/responses`（OpenAI Responses 协议变体）流式调用。
4. 桌面端与 CLI 都提供登录入口；凭据存 Windows 凭据管理器，与既有 API key 通道一致。

## 客户端身份与风险

OpenAI 没有公开的 OAuth 应用注册渠道，第三方集成通行做法是复用 codex CLI 的 client_id：

- `client_id = app_EMoamEEZ73f0CkXaXp7hrann`，`originator = codex_cli_rs`（OpenAI 后端按此识别客户端）。
- 授权 URL 附加 `id_token_add_organizations=true`、`codex_cli_simplified_flow=true`、`originator=codex_cli_rs`。
- 理论上存在被 OpenAI 风控/封禁的边缘风险；已与用户确认采用该策略。

## OAuth 流程

常量：issuer `https://auth.openai.com`；authorize `{issuer}/oauth/authorize`；token `{issuer}/oauth/token`；revoke `{issuer}/oauth/revoke`；scope `openid profile email offline_access api.connectors.read api.connectors.invoke`；redirect `http://127.0.0.1:{port}/auth/callback`，端口 1455 被占回落 1457。

1. **发起**：生成 PKCE（64 字节随机 → base64url 无填充 `code_verifier`，S256 `code_challenge`）与 state（32 字节随机 base64url）；构造授权 URL；启动本地回环回调服务器。
2. **回调**：浏览器登录成功后回跳 `/auth/callback?code=...&state=...`；服务器严格比对 state，换 token（`POST {issuer}/oauth/token`，**form 编码**：`grant_type=authorization_code` + `code_verifier`），向浏览器返回成功页并结束。
3. **持久化**：解析 id_token JWT（base64url，无签名验证）取 `email`、`https://api.openai.com/auth` 下的 `chatgpt_account_id`、`chatgpt_plan_type`；access_token 的 `exp` 为过期时间；写入凭据管理器。
4. **刷新**：调用前 access_token 剩余 ≤5 分钟则刷新（`POST {issuer}/oauth/token`，**JSON 编码**：`grant_type=refresh_token` + `refresh_reason`）；失败分类：`refresh_token_expired/reused/invalidated` 或 400 `invalid_grant` 为永久失败（要求重新登录），其余可重试。
5. **登出**：先调 `/oauth/revoke` 吊销 refresh_token，再删除本地凭据；吊销失败不阻塞本地删除。

## 存储设计

- `TokenBundle`：`access_token / refresh_token / account_id / expires_at / email / plan_type`（id_token 解析后不保留）。
- `TokenStore` trait（load/save/delete），默认实现落 `harness_platform::secrets`；测试注入内存实现，不碰真实凭据管理器。
- 凭据管理器单值上限 2560 字节，JWT 体积可能超限，**拆分两个 reference**：`provider/chatgpt`（access_token + account_id + 元数据 JSON）、`provider/chatgpt.refresh`（refresh_token）。刷新时先写 refresh 后写 access，避免半状态。
- 凭据引用挂在内置供应商 `chatgpt`（`credential_ref = provider/chatgpt`），复用既有 save/build 的 secret 通道之外的新路径：登录/刷新直接经 `TokenStore` 写入，不经 `provider_save` 的 `secret` 参数。

## 请求协议（ChatGPT 后端 Responses 变体）

- URL：`POST https://chatgpt.com/backend-api/codex/responses`，`Accept: text/event-stream`，必须流式。
- 请求体差异（相对标准 Responses）：`store: false` 恒定；`include: ["reasoning.encrypted_content"]` 恒定；基础指令不以顶层 `instructions` 发送，而是作为 `developer` 消息置入 `input`。
- Header：`Authorization: Bearer {access_token}`、`chatgpt-account-id: {account_id}`、`originator: codex_cli_rs`、`session-id`（每次请求随机 UUID）、`User-Agent: codex_cli_rs/{version}`。
- SSE 事件与标准 Responses API 一致，现有 `SseDecoder` 复用。
- 内置模型目录照搬 codex `models-manager/models.json` 当前 `show_in_picker` 模型与 reasoning 档位（low/medium/high/xhigh），不做 `/models` 动态拉取（`fetch_available_models` 对该供应商禁用）。

## 契约与服务层改动

- `harness-contracts`：新增 `ProviderAuth`（`ApiKey`（default）| `ChatGptOAuth`）；`ProviderInfo`、`ModelConfig` 增 `auth` 字段（`#[serde(default)]` 向后兼容，SCHEMA_VERSION 不变）。
- `harness-runtime::chatgpt`（新模块）：常量、PKCE/state/URL 构造、JWT 解析、TokenBundle、TokenStore trait 及默认实现、回调服务器（tokio TcpListener 手写 minimal HTTP）、token 交换/刷新/吊销、`ensure_access_token`。
- `providers.rs`：`builtin_providers()` 增 `chatgpt` 条目（唯一 Responses endpoint、`auth: ChatGptOAuth`、内置模型）；`builtin_model_config` 按供应商取对应 endpoint（chatgpt 用 Responses，其余仍 Anthropic）；`require_credential` 对 chatgpt 改为"有 TokenBundle"；`build_model_with` 对 OAuth 供应商读 bundle → 刷新 → 产出 access_token 与 account-id。
- `model.rs`：`HttpModel` 增 `extra_headers: Vec<(String, String)>`，请求时附加；`model/encoding.rs` Responses 编码按 `config.auth` 分派 ChatGPT 变体（不影响 bigmodel 既有 Responses 路径）。

## 桌面与 CLI

- CLI：顶层 `harness login`（起回调服务器 → `webbrowser` 打开浏览器 → 等待完成）、`harness login status`（邮箱/套餐/过期时间或未登录）、`harness logout`。
- 桌面命令（specta 注册，secret 永不回传）：`chatgpt_login_start`（返回 auth_url 并用 tauri-plugin-opener 打开系统浏览器）、`chatgpt_login_status`（轮询：pending/completed/failed/cancelled）、`chatgpt_login_cancel`、`chatgpt_logout`。
- 前端 `ModelPanel.tsx`：chatgpt 供应商卡片分支渲染——无 API key 输入框与"从 API 获取模型"按钮，改为"使用 ChatGPT 账号登录"按钮 + 登录中轮询 + 已登录信息（邮箱、套餐）+ 登出按钮。

## 测试

- chatgpt.rs 单测：PKCE/state 格式、授权 URL 参数断言、JWT claims 解析（自制 alg=none JWT）、TokenBundle serde、刷新窗口纯函数、刷新失败分类。
- 回调服务器 e2e：mock issuer（TcpListener 假 `/oauth/token`）+ reqwest 模拟浏览器回调，断言保存；state 不匹配 400；`/cancel`；端口回落 1457。
- build 链路：内存 TokenStore → HttpModel 携带 access_token + account-id；缺凭据报 `MODEL_CREDENTIAL`。
- encoding：ChatGPT 变体请求体断言（store/include/developer）。
- 前端 vitest：chatgpt 卡片渲染、登录按钮触发命令、轮询与登出（mock `command`）。

## 风险点

- 复用 codex client_id 的风控风险（已确认接受）。
- ChatGPT 后端为私有协议，字段随 codex 版本演进可能变化，以当前 codex 源码为准。
- 凭据管理器 2560 字节上限以拆分存储规避；刷新写回顺序保证可恢复。
- 秘钥仅落凭据管理器，前端与日志不回显。
