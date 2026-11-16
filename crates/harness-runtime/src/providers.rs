//! Shared supplier configuration and authoritative route resolution for every carrier.
use crate::model::{HttpModel, ModelMessage, ModelProvider, transport_error};
use harness_contracts::*;
use harness_storage::Store;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

/// HTTP 连接复用开关（KV，缺省开启；关闭时每次模型调用新建 HTTP 客户端）。
pub(crate) const HTTP_CONNECTION_REUSE_KEY: &str = "app.http_connection_reuse";

/// 内置供应商目录（deepseek、bigmodel）及其只读模型清单。
/// 目录身份在升级间保持稳定；用户只能通过个人覆盖层启用/禁用，不能改写。
/// 内置模型目录条目：id、显示名、effort 档位、上下文窗口（tokens）、最大输出（tokens）；第 6 项为图片输入能力（Some=官方文档已核实，None=未知）。
type BuiltinCatalogEntry<'a> = (
    &'a str,
    &'a str,
    &'a [&'a str],
    u32,
    Option<u32>,
    Option<bool>,
);

pub fn builtin_providers() -> Vec<ProviderInfo> {
    // 模型上下文窗口与最大输出为公开规格预填值（tokens），集中在此便于校正。
    // 最大输出按官方模型页规格：deepseek 系 384K，智谱系 128K（思维链与正文共享该上限）。
    let deepseek_models: [BuiltinCatalogEntry; 2] = [
        (
            "deepseek-flash",
            "DeepSeek Flash",
            &["disabled", "enabled"],
            128_000,
            Some(393_216),
            Some(true),
        ),
        (
            "deepseek-v4-pro",
            "DeepSeek V4 Pro",
            &["disabled", "enabled"],
            128_000,
            Some(393_216),
            Some(false),
        ),
    ];
    // 上下文窗口按官方模型页公开规格：glm-5.2/5.3/5.3-flash 为 1M，5-turbo 沿用 200k。
    let bigmodel_models: [BuiltinCatalogEntry; 4] = [
        (
            "glm-5.3",
            "GLM-5.3",
            &["low", "high", "max"],
            1_000_000,
            Some(131_072),
            Some(false),
        ),
        (
            "glm-5.3-flash",
            "GLM-5.3 Flash",
            &["low", "high", "max"],
            1_000_000,
            Some(131_072),
            Some(true),
        ),
        (
            "glm-5.2",
            "GLM-5.2",
            &["disabled", "high", "max"],
            1_000_000,
            Some(131_072),
            Some(false),
        ),
        (
            "glm-5-turbo",
            "GLM-5 Turbo",
            &["disabled", "enabled"],
            200_000,
            Some(131_072),
            Some(false),
        ),
    ];
    let mut providers: Vec<ProviderInfo> = [
        (
            "deepseek",
            "DeepSeek",
            "deepseek-flash",
            deepseek_models.as_slice(),
            [
                "https://api.deepseek.com",
                "https://api.deepseek.com",
                "https://api.deepseek.com/anthropic",
            ],
        ),
        (
            "bigmodel",
            "智谱 Coding Plan",
            "glm-5.3",
            bigmodel_models.as_slice(),
            [
                "https://open.bigmodel.cn/api/coding/paas/v4",
                "https://open.bigmodel.cn/api/v1",
                "https://open.bigmodel.cn/api/anthropic",
            ],
        ),
    ]
    .into_iter()
    .map(|(id, name, model, models, urls)| ProviderInfo {
        compatibility: if id == "deepseek" {
            ProviderCompatibility::DeepSeek
        } else {
            ProviderCompatibility::BigModel
        },
        auth: ProviderAuth::ApiKey,
        id: id.into(),
        name: name.into(),
        builtin: true,
        default_model: model.into(),
        credential_ref: format!("provider/{id}"),
        proxy: None,
        endpoints: [
            ApiMode::ChatCompletions,
            ApiMode::Responses,
            ApiMode::Anthropic,
        ]
        .into_iter()
        .zip(urls)
        .map(|(api_mode, base_url)| ProtocolEndpoint {
            api_mode,
            base_url: base_url.into(),
        })
        .collect(),
        enabled: true,
        builtin_models: models
            .iter()
            .map(
                |(id, name, efforts, window, max_output, vision)| BuiltinModel {
                    id: (*id).into(),
                    name: (*name).into(),
                    efforts: efforts.iter().map(|e| (*e).into()).collect(),
                    context_window: Some(*window),
                    max_output_tokens: *max_output,
                    supports_vision: *vision,
                },
            )
            .collect(),
        disabled_models: Vec::new(),
    })
    .collect();
    providers.push(chatgpt_provider());
    providers
}

/// ChatGPT 账号供应商：OAuth 登录（非 API key），唯一 Responses 端点指向 ChatGPT 后端。
/// 模型目录与 effort 档位照搬 codex `models-manager/models.json` 的 picker 可见条目。
fn chatgpt_provider() -> ProviderInfo {
    // codex 后端对未知模型统一按 272k 窗口下发（models-manager fallback），此处沿用。
    // 输出上限为 None：ChatGPT 后端不消费 max_output_tokens，发送无意义。
    let models: [BuiltinCatalogEntry; 9] = [
        (
            "gpt-6.1-sol",
            "GPT-6.1-Sol",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-6-astra",
            "GPT-6-Astra",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-6-sol",
            "GPT-6-Sol",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-6-luna",
            "GPT-6-Luna",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-5.6-sol",
            "GPT-5.6-Sol",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-6.1-terra",
            "GPT-6.1 Terra",
            &["low", "medium", "high", "xhigh", "max"],
            500_000,
            None,
            Some(true),
        ),
        (
            "gpt-5.6-terra",
            "GPT-5.6-Terra",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-5.6-luna",
            "GPT-5.6-Luna",
            &["low", "medium", "high", "xhigh", "max"],
            272_000,
            None,
            Some(true),
        ),
        (
            "gpt-5.5",
            "GPT-5.5",
            &["low", "medium", "high", "xhigh"],
            272_000,
            None,
            Some(true),
        ),
    ];
    ProviderInfo {
        compatibility: ProviderCompatibility::Standard,
        auth: ProviderAuth::ChatGptOAuth,
        id: "chatgpt".into(),
        name: "ChatGPT 账号".into(),
        builtin: true,
        default_model: "gpt-6.1-sol".into(),
        credential_ref: crate::chatgpt::CREDENTIAL_MAIN.into(),
        proxy: None,
        enabled: true,
        endpoints: vec![ProtocolEndpoint {
            api_mode: ApiMode::Responses,
            base_url: crate::chatgpt::CHATGPT_BASE_URL.into(),
        }],
        builtin_models: models
            .iter()
            .map(
                |(id, name, efforts, window, max_output, vision)| BuiltinModel {
                    id: (*id).into(),
                    name: (*name).into(),
                    efforts: efforts.iter().map(|e| (*e).into()).collect(),
                    context_window: Some(*window),
                    max_output_tokens: *max_output,
                    supports_vision: *vision,
                },
            )
            .collect(),
        disabled_models: Vec::new(),
    }
}

/// 由内置目录条目合成的可路由模型配置；api_mode 取供应商协议端点
/// （既有目录固定 Anthropic，chatgpt 仅有 Responses）。
fn builtin_model_config(provider: &ProviderInfo, model: &BuiltinModel) -> Option<ModelConfig> {
    let endpoint = provider
        .endpoints
        .iter()
        .find(|e| e.api_mode == ApiMode::Anthropic)
        .or_else(|| provider.endpoints.first())?;
    Some(ModelConfig {
        provider_id: provider.id.clone(),
        auth: provider.auth,
        id: format!("builtin:{}:{}", provider.id, model.id),
        name: model.name.clone(),
        base_url: endpoint.base_url.clone(),
        model: model.id.clone(),
        credential_ref: provider.credential_ref.clone(),
        proxy: provider.proxy.clone(),
        compatibility: provider.compatibility,
        api_mode: endpoint.api_mode,
        max_output_tokens: model.max_output_tokens,
        context_window: model.context_window,
        supports_vision: model.supports_vision,
        efforts: model.efforts.clone(),
        effort: None,
        enabled: !provider.disabled_models.contains(&model.id),
        builtin: true,
    })
}

/// 内置供应商的模型必须配置密钥才能执行；自定义供应商允许空密钥（请求时由端点拒绝）。
/// OAuth 供应商的凭据由登录流程维护，不经此通道（见 `oauth_material`）。
fn require_credential(config: &ModelConfig, secret: &str) -> AppResult<()> {
    if config.auth == ProviderAuth::ChatGptOAuth {
        return Err(AppError::new(
            "MODEL_CREDENTIAL",
            "请先使用 ChatGPT 账号登录",
        ));
    }
    if secret.is_empty()
        && builtin_providers()
            .iter()
            .any(|p| p.id == config.provider_id)
    {
        return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
    }
    Ok(())
}

/// ChatGPT OAuth 供应商的请求材料：刷新后的 access_token 与身份/路由 header。
/// 未登录报 `MODEL_CREDENTIAL`；凭据临近过期时自动刷新并回写存储。
async fn oauth_material(
    token_store: &dyn crate::chatgpt::TokenStore,
    config: &ModelConfig,
    now: i64,
) -> AppResult<(String, Vec<(String, String)>)> {
    use crate::chatgpt::{ISSUER, ORIGINATOR, ensure_access_token, load_bundle};
    let bundle = load_bundle(token_store)?
        .ok_or_else(|| AppError::new("MODEL_CREDENTIAL", "请先使用 ChatGPT 账号登录"))?;
    let bundle =
        ensure_access_token(token_store, bundle, ISSUER, now, config.proxy.as_deref()).await?;
    let extra_headers = vec![
        ("chatgpt-account-id".to_string(), bundle.account_id.clone()),
        ("originator".to_string(), ORIGINATOR.to_string()),
        ("session-id".to_string(), new_id()),
    ];
    Ok((bundle.access_token, extra_headers))
}

/// 错误提示用的启用模型 id 清单（与 CLI 侧报错风格对齐）。
fn enabled_id_list(models: &[ModelConfig]) -> String {
    models
        .iter()
        .filter(|model| model.enabled)
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 应用本次请求的 effort 覆盖；档位必须在模型支持列表内，None 表示清除覆盖、跟随供应商默认。
fn apply_effort(mut config: ModelConfig, effort: Option<String>) -> AppResult<ModelConfig> {
    match effort {
        Some(value) => {
            if config.efforts.is_empty() || !config.efforts.contains(&value) {
                return Err(AppError::new(
                    "MODEL_EFFORT",
                    format!(
                        "推理档位 {value} 不受支持，可选：{}",
                        config.efforts.join(", ")
                    ),
                ));
            }
            config.effort = Some(value);
        }
        None => config.effort = None,
    }
    Ok(config)
}
fn invalid(message: &str) -> AppError {
    AppError::new("MODEL_CONFIG", message)
}

/// 拉取端点的模型列表：OpenAI 兼容协议请求 `{base}/models`（Bearer），
/// Anthropic 协议请求 `{base}/v1/models`（x-api-key），返回去重排序的模型 ID。
pub(crate) async fn fetch_model_ids(
    base_url: &str,
    api_mode: ApiMode,
    secret: &str,
    proxy: Option<&str>,
) -> AppResult<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let client = http_client(proxy)?;
    let request = match api_mode {
        ApiMode::Anthropic => {
            let url = if base.ends_with("/v1") {
                format!("{base}/models")
            } else {
                format!("{base}/v1/models")
            };
            client
                .get(url)
                .header("anthropic-version", "2023-06-01")
                .header("x-api-key", secret)
        }
        _ => client.get(format!("{base}/models")).bearer_auth(secret),
    };
    let response = request
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(transport_error)?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "MODEL_FETCH",
            format!("获取模型列表失败：HTTP {}", response.status()),
        ));
    }
    let value: serde_json::Value = response.json().await.map_err(transport_error)?;
    let mut ids: Vec<String> = value
        .get("data")
        .and_then(|data| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(AppError::new("MODEL_FETCH", "端点未返回模型列表"));
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}
/// 构建带超时、禁重定向与可选代理的 HTTP 客户端；模型列表与额度请求共用。
fn http_client(proxy: Option<&str>) -> AppResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy.filter(|p| !p.is_empty()) {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy).map_err(|_| AppError::new("MODEL_PROXY", "代理地址无效"))?,
        );
    }
    builder.build().map_err(transport_error)
}
/// Coding Plan 剩余额度端点；monitor 接口要求 authorization 直传完整凭据（无 Bearer 前缀）。
const BIGMODEL_USAGE_QUOTA_URL: &str = "https://bigmodel.cn/api/monitor/usage/quota/limit";

/// 请求 monitor 额度接口；超时与重定向策略与模型列表一致。
async fn fetch_usage_quota_json(
    url: &str,
    secret: &str,
    proxy: Option<&str>,
) -> AppResult<serde_json::Value> {
    let client = http_client(proxy)?;
    let response = client
        .get(url)
        .header("authorization", secret)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(transport_error)?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "MODEL_USAGE",
            format!("获取额度失败：HTTP {}", response.status()),
        ));
    }
    response.json().await.map_err(transport_error)
}

/// 将 quota/limit 响应映射为剩余额度快照；envelope 失败时报 MODEL_USAGE。
fn map_usage_quota(value: serde_json::Value) -> AppResult<ProviderUsageSnapshot> {
    let success = value.get("success").and_then(|v| v.as_bool());
    let code = value.get("code").and_then(|v| v.as_i64());
    if success == Some(false) || code.is_some_and(|c| c != 0 && c != 200) {
        let msg = value
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("接口返回异常");
        return Err(AppError::new("MODEL_USAGE", format!("获取额度失败：{msg}")));
    }
    let data = value.get("data");
    let plan_name = data
        .and_then(|d| d.get("level"))
        .and_then(|v| v.as_str())
        .and_then(normalize_plan_level);
    let mut cards = Vec::new();
    if let Some(limits) = data
        .and_then(|d| d.get("limits"))
        .and_then(|v| v.as_array())
    {
        // 按固定顺序匹配三张卡（type/unit 约定一致），未知条目忽略。
        for (kind, quota_type, unit) in [
            (UsageCardKind::FiveHour, "TOKENS_LIMIT", 3),
            (UsageCardKind::Weekly, "TOKENS_LIMIT", 6),
            (UsageCardKind::Tools, "TIME_LIMIT", 5),
        ] {
            let Some(limit) = limits.iter().find(|l| {
                l.get("type").and_then(|v| v.as_str()) == Some(quota_type)
                    && l.get("unit").and_then(|v| v.as_i64()) == Some(unit)
            }) else {
                continue;
            };
            cards.push(ProviderUsageCard {
                kind,
                remaining_percent: remaining_percent(limit),
                reset_at: limit.get("nextResetTime").and_then(|v| v.as_i64()),
            });
        }
    }
    Ok(ProviderUsageSnapshot { plan_name, cards })
}

/// 接口的 percentage 为已用百分比，反转为剩余；缺失时回退 remaining/usage。
fn remaining_percent(limit: &serde_json::Value) -> Option<f64> {
    if let Some(used) = limit.get("percentage").and_then(|v| v.as_f64()) {
        return Some((100.0 - used.clamp(0.0, 100.0)).max(0.0));
    }
    let remaining = limit.get("remaining").and_then(|v| v.as_f64());
    let total = match limit.get("usage").and_then(|v| v.as_f64()) {
        Some(total) => Some(total),
        None => Some(limit.get("currentValue").and_then(|v| v.as_f64())? + remaining?),
    };
    match (remaining, total) {
        (Some(remaining), Some(total)) if total > 0.0 => {
            Some((remaining / total * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    }
}

/// 将 quota level 归一化为套餐显示名；已知档位映射为 GLM Coding 系列，其余原样保留。
fn normalize_plan_level(level: &str) -> Option<String> {
    let trimmed = level.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "lite" | "pro" | "max" => {
            let mut chars = trimmed.chars();
            let (first, rest) = (chars.next().unwrap(), chars.as_str());
            Some(format!("GLM Coding {}{}", first.to_uppercase(), rest))
        }
        _ => Some(trimmed.to_string()),
    }
}

/// ChatGPT 订阅用量端点；与模型基址同域（`CHATGPT_BASE_URL` 的 `/codex` 上一级），路径规则见 codex backend-client。
const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// 拉取 ChatGPT 订阅用量；凭据由 OAuth 材料提供（自动刷新 access token）。
async fn fetch_chatgpt_usage_json(
    token_store: &dyn crate::chatgpt::TokenStore,
    config: &ModelConfig,
    now: i64,
) -> AppResult<serde_json::Value> {
    let (secret, extra_headers) = oauth_material(token_store, config, now).await?;
    let client = http_client(config.proxy.as_deref())?;
    let mut request = client
        .get(CHATGPT_USAGE_URL)
        .bearer_auth(secret)
        .timeout(std::time::Duration::from_secs(20));
    for (name, value) in extra_headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(transport_error)?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "MODEL_USAGE",
            format!("获取额度失败：HTTP {}", response.status()),
        ));
    }
    response.json().await.map_err(transport_error)
}

/// 将 /wham/usage 响应映射为剩余额度快照；used_percent 为已用口径，展示前反转。
fn map_chatgpt_usage(value: serde_json::Value, now: i64) -> AppResult<ProviderUsageSnapshot> {
    let plan_name = value
        .get("plan_type")
        .and_then(|v| v.as_str())
        .and_then(chatgpt_plan_name);
    let windows = value.get("rate_limit");
    let mut cards = Vec::new();
    for (kind, key) in [
        (UsageCardKind::FiveHour, "primary_window"),
        (UsageCardKind::Weekly, "secondary_window"),
    ] {
        let Some(window) = windows.and_then(|w| w.get(key)) else {
            continue;
        };
        let used = window
            .get("used_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .clamp(0.0, 100.0);
        let reset_at = window
            .get("reset_at")
            .and_then(|v| v.as_i64())
            .or_else(|| {
                window
                    .get("reset_after_seconds")
                    .and_then(|v| v.as_i64())
                    .map(|after| now + after)
            })
            .map(|seconds| seconds * 1000);
        cards.push(ProviderUsageCard {
            kind,
            remaining_percent: Some(100.0 - used),
            reset_at,
        });
    }
    Ok(ProviderUsageSnapshot { plan_name, cards })
}

/// plan_type 归一化为套餐显示名（首字母大写并带 ChatGPT 前缀）。
fn chatgpt_plan_name(plan_type: &str) -> Option<String> {
    let trimmed = plan_type.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let (first, rest) = (chars.next().unwrap(), chars.as_str());
    Some(format!("ChatGPT {}{}", first.to_uppercase(), rest))
}
fn decode(e: impl std::fmt::Display) -> AppError {
    AppError::new("MODEL_CONFIG", e.to_string())
}
fn validate_url(raw: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(raw).map_err(|_| invalid("请输入有效 HTTP(S) 地址"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("服务地址必须为不含凭据、查询和片段的 HTTP(S) 地址"));
    }
    Ok(())
}
#[derive(Clone)]
pub struct ModelService {
    store: Store,
    gate: Arc<Mutex<()>>,
    /// ChatGPT OAuth 凭据存储；默认落系统凭据管理器，测试注入内存实现。
    token_store: Arc<dyn crate::chatgpt::TokenStore>,
}
impl ModelService {
    pub fn new(store: Store) -> Self {
        Self::with_token_store(store, Arc::new(crate::chatgpt::PlatformTokenStore))
    }
    pub fn with_token_store(
        store: Store,
        token_store: Arc<dyn crate::chatgpt::TokenStore>,
    ) -> Self {
        Self {
            store,
            gate: Arc::new(Mutex::new(())),
            token_store,
        }
    }
    async fn read<T: DeserializeOwned>(&self, key: &str) -> AppResult<Vec<T>> {
        serde_json::from_value(self.store.get(key).await?.unwrap_or(json!([]))).map_err(decode)
    }
    async fn providers(&self) -> AppResult<Vec<ProviderInfo>> {
        let mut all = builtin_providers();
        for saved in self.read::<ProviderInfo>("providers").await? {
            if let Some(builtin) = all.iter_mut().find(|p| p.id == saved.id) {
                // Built-in catalog identity and URLs are immutable; personal layer
                // may only toggle visibility (proxy / enabled / disabled models).
                builtin.proxy = saved.proxy;
                builtin.enabled = saved.enabled;
                builtin.disabled_models = saved.disabled_models;
            } else {
                all.push(saved);
            }
        }
        Ok(all)
    }
    pub async fn list_providers(&self) -> AppResult<Vec<ProviderInfo>> {
        let _guard = self.gate.lock().await;
        self.providers().await
    }
    pub async fn save_provider(
        &self,
        mut config: ProviderInfo,
        secret: Option<String>,
    ) -> AppResult<ProviderInfo> {
        let _guard = self.gate.lock().await;
        let mut all = self.read::<ProviderInfo>("providers").await?;
        let existing = all.iter().find(|p| p.id == config.id).cloned();
        if let Some(builtin) = builtin_providers().into_iter().find(|p| p.id == config.id) {
            if config.name != builtin.name
                || config.endpoints != builtin.endpoints
                || !config.builtin
                || config.default_model != builtin.default_model
                || config.compatibility != builtin.compatibility
            {
                return Err(invalid(
                    "内置供应商名称、模型和端点不可修改，请复制为自定义供应商",
                ));
            }
            config.credential_ref = builtin.credential_ref;
            // 个人覆盖层只保留启停状态；目录身份与模型清单始终以程序内置为准。
            config.builtin_models = Vec::new();
        } else {
            if config.id.is_empty() {
                config.id = new_id();
            }
            config.builtin = false;
            // Only migration can retain a legacy model credential reference.
            config.credential_ref = existing
                .map(|p| p.credential_ref)
                .unwrap_or_else(|| format!("provider/{}", config.id));
            config.endpoints.retain(|e| !e.base_url.trim().is_empty());
            if config.name.trim().is_empty()
                || config.default_model.trim().is_empty()
                || config.endpoints.is_empty()
            {
                return Err(invalid("请填写名称、默认模型和至少一个协议端点"));
            }
            let mut modes = Vec::new();
            for endpoint in &config.endpoints {
                validate_url(&endpoint.base_url)?;
                if modes.contains(&endpoint.api_mode) {
                    return Err(invalid("协议端点不能重复"));
                }
                modes.push(endpoint.api_mode);
            }
        }
        for model in self
            .read::<ModelConfig>("models")
            .await?
            .iter()
            .filter(|m| m.provider_id == config.id)
        {
            if !config
                .endpoints
                .iter()
                .any(|e| e.api_mode == model.api_mode)
            {
                return Err(invalid("该协议仍有模型引用，请先删除相关模型"));
            }
        }
        if let Some(proxy) = config.proxy.as_ref().filter(|p| !p.is_empty()) {
            reqwest::Proxy::all(proxy).map_err(|_| invalid("代理地址无效"))?;
        }
        if let Some(secret) = secret.filter(|s| !s.is_empty()) {
            harness_platform::set_secret(&config.credential_ref, &secret)?;
        }
        all.retain(|p| p.id != config.id);
        all.push(config.clone());
        self.store.set("providers", &json!(all)).await?;
        Ok(config)
    }
    pub async fn copy_provider(&self, id: &str) -> AppResult<ProviderInfo> {
        let mut config = self
            .list_providers()
            .await?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        config.id = String::new();
        config.name = format!("{} 副本", config.name);
        config.builtin = false;
        config.credential_ref = String::new();
        self.save_provider(config, None).await
    }
    pub async fn remove_provider(&self, id: &str) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        if builtin_providers().iter().any(|p| p.id == id) {
            return Err(invalid("内置供应商不可删除"));
        }
        let mut all = self.read::<ProviderInfo>("providers").await?;
        if !all.iter().any(|p| p.id == id) {
            return Err(invalid("供应商不存在"));
        }
        // Require explicit model removal so a supplier deletion cannot leave dangling routes.
        if self
            .read::<ModelConfig>("models")
            .await?
            .iter()
            .any(|m| m.provider_id == id)
        {
            return Err(invalid("请先删除此供应商的模型"));
        }
        all.retain(|p| p.id != id);
        self.store.set("providers", &json!(all)).await
    }
    fn resolve(mut model: ModelConfig, providers: &[ProviderInfo]) -> AppResult<ModelConfig> {
        let supplier = providers
            .iter()
            .find(|p| p.id == model.provider_id)
            .ok_or_else(|| invalid("请选择供应商"))?;
        let endpoint = supplier
            .endpoints
            .iter()
            .find(|p| p.api_mode == model.api_mode)
            .ok_or_else(|| invalid("供应商未配置该协议"))?;
        model.compatibility = supplier.compatibility;
        model.auth = supplier.auth;
        model.base_url = endpoint.base_url.clone();
        model.proxy = supplier.proxy.clone();
        model.credential_ref = supplier.credential_ref.clone();
        Ok(model)
    }
    /// 全量模型视图：内置供应商的目录模型（含停用标记）+ 已保存的自定义模型。
    /// 条目的 `enabled` 只表示自身状态；供应商停用由 `build_model` 拦截。
    pub async fn list_models(&self) -> AppResult<Vec<ModelConfig>> {
        let _guard = self.gate.lock().await;
        let suppliers = self.providers().await?;
        let mut all: Vec<ModelConfig> = Vec::new();
        for supplier in &suppliers {
            for model in &supplier.builtin_models {
                if let Some(config) = builtin_model_config(supplier, model) {
                    all.push(config);
                }
            }
        }
        for model in self.read::<ModelConfig>("models").await? {
            all.push(Self::resolve(model, &suppliers)?);
        }
        Ok(all)
    }
    pub async fn save_model(
        &self,
        mut config: ModelConfig,
        secret: Option<String>,
    ) -> AppResult<ModelConfig> {
        let _guard = self.gate.lock().await;
        if config.name.trim().is_empty()
            || config.model.trim().is_empty()
            || config.max_output_tokens == Some(0)
        {
            return Err(invalid("名称、模型不能为空，输出上限必须大于0"));
        }
        if config.builtin {
            return Err(invalid("内置模型不可修改，请在模型设置中启用或禁用"));
        }
        for value in &config.efforts {
            if !KNOWN_EFFORTS.contains(&value.as_str()) {
                return Err(invalid(&format!("推理档位 {value} 不受支持")));
            }
        }
        if config
            .effort
            .as_ref()
            .is_some_and(|effort| config.efforts.is_empty() || !config.efforts.contains(effort))
        {
            return Err(invalid("默认档位必须在该模型支持的档位列表内"));
        }
        if config.id.is_empty() {
            config.id = new_id();
        }
        let suppliers = self.providers().await?;
        if let Some(builtin) = suppliers
            .iter()
            .find(|p| p.builtin && p.id == config.provider_id)
            && let Some(endpoint) = builtin
                .endpoints
                .iter()
                .find(|e| e.api_mode == config.api_mode)
            && !config.base_url.is_empty()
            && config.base_url.trim_end_matches('/') != endpoint.base_url.trim_end_matches('/')
        {
            return Err(invalid("内置供应商端点不可覆盖"));
        }
        config = Self::resolve(config, &suppliers)?;
        if let Some(secret) = secret.filter(|s| !s.is_empty()) {
            harness_platform::set_secret(&config.credential_ref, &secret)?;
        }
        let mut all = self.read::<ModelConfig>("models").await?;
        all.retain(|m| m.id != config.id);
        all.push(config.clone());
        self.store.set("models", &json!(all)).await?;
        Ok(config)
    }
    pub async fn remove_model(&self, id: &str) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        let mut all = self.read::<ModelConfig>("models").await?;
        all.retain(|m| m.id != id);
        self.store.set("models", &json!(all)).await
    }
    pub async fn build_model(&self, id: &str) -> AppResult<Arc<dyn ModelProvider>> {
        self.build_model_with(id, None).await
    }
    /// 构建可执行模型；`effort` 为本次请求的档位覆盖（必须在模型支持列表内）。
    pub async fn build_model_with(
        &self,
        id: &str,
        effort: Option<String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        let catalog = self.list_models().await?;
        let mut config = catalog
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| {
                AppError::new(
                    "MODEL_NOT_FOUND",
                    format!("模型 {id} 不存在。可用模型：{}", enabled_id_list(&catalog)),
                )
            })?
            .clone();
        if !config.enabled {
            return Err(AppError::new("MODEL_DISABLED", "模型已停用"));
        }
        if !self
            .providers()
            .await?
            .iter()
            .any(|p| p.id == config.provider_id && p.enabled)
        {
            return Err(AppError::new("MODEL_DISABLED", "供应商已停用"));
        }
        config = apply_effort(config, effort)?;
        let connection_reuse = self.http_connection_reuse().await;
        if config.auth == ProviderAuth::ChatGptOAuth {
            let (secret, extra_headers) =
                oauth_material(&*self.token_store, &config, chrono::Utc::now().timestamp()).await?;
            return Ok(Arc::new(HttpModel {
                config,
                secret,
                extra_headers,
                stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
                connection_reuse,
            }));
        }
        let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
        require_credential(&config, &secret)?;
        Ok(Arc::new(HttpModel {
            config,
            secret,
            extra_headers: Vec::new(),
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
            connection_reuse,
        }))
    }
    /// HTTP 连接复用开关（KV `app.http_connection_reuse`，缺省开启）。
    pub async fn http_connection_reuse(&self) -> bool {
        self.store
            .get(HTTP_CONNECTION_REUSE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }
    pub async fn set_http_connection_reuse(&self, enabled: bool) -> AppResult<()> {
        self.store
            .set(HTTP_CONNECTION_REUSE_KEY, &json!(enabled))
            .await
    }
    /// 内置目录模型的启用/禁用；写个人覆盖层，内置目录本身不变。
    pub async fn set_builtin_model_enabled(
        &self,
        provider_id: &str,
        model_id: &str,
        enabled: bool,
    ) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        let supplier = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        if !supplier.builtin_models.iter().any(|m| m.id == model_id) {
            return Err(invalid("内置模型不存在"));
        }
        let mut all = self.read::<ProviderInfo>("providers").await?;
        let saved = all.iter_mut().find(|p| p.id == provider_id);
        match saved {
            Some(saved) => {
                saved.disabled_models.retain(|id| id != model_id);
                if !enabled {
                    saved.disabled_models.push(model_id.to_string());
                }
            }
            None => {
                all.push(ProviderInfo {
                    compatibility: Default::default(),
                    auth: Default::default(),
                    id: provider_id.to_string(),
                    name: String::new(),
                    builtin: true,
                    endpoints: Vec::new(),
                    default_model: String::new(),
                    credential_ref: String::new(),
                    proxy: supplier.proxy.clone(),
                    enabled: true,
                    builtin_models: Vec::new(),
                    disabled_models: if enabled {
                        Vec::new()
                    } else {
                        vec![model_id.to_string()]
                    },
                });
            }
        }
        self.store.set("providers", &json!(all)).await
    }
    /// 按供应商与协议从远端拉取可用模型列表（GET /models），供设置页选择后手动保存。
    pub async fn fetch_available_models(
        &self,
        provider_id: &str,
        api_mode: ApiMode,
    ) -> AppResult<Vec<String>> {
        let _guard = self.gate.lock().await;
        let supplier = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        if supplier.auth == ProviderAuth::ChatGptOAuth {
            return Err(invalid(
                "ChatGPT 供应商的模型目录随程序内置，不支持从 API 获取",
            ));
        }
        let supplier = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        let endpoint = supplier
            .endpoints
            .iter()
            .find(|e| e.api_mode == api_mode)
            .ok_or_else(|| invalid("供应商未配置该协议"))?;
        let secret = harness_platform::get_secret(&supplier.credential_ref)?.unwrap_or_default();
        if secret.is_empty() {
            return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
        }
        fetch_model_ids(
            &endpoint.base_url,
            api_mode,
            &secret,
            supplier.proxy.as_deref(),
        )
        .await
    }
    /// 拉取供应商套餐与剩余额度（内置智谱 Coding Plan 与 ChatGPT 订阅）；快照即查即返，不落库。
    pub async fn fetch_usage(&self, provider_id: &str) -> AppResult<ProviderUsageSnapshot> {
        let _guard = self.gate.lock().await;
        let supplier = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        if !supplier.builtin {
            return Err(invalid("该供应商不支持额度查询"));
        }
        if supplier.auth == ProviderAuth::ChatGptOAuth {
            let now = chrono::Utc::now().timestamp();
            let config = ModelConfig {
                proxy: supplier.proxy.clone(),
                ..Default::default()
            };
            let value = fetch_chatgpt_usage_json(&*self.token_store, &config, now).await?;
            return map_chatgpt_usage(value, now);
        }
        if supplier.compatibility != ProviderCompatibility::BigModel {
            return Err(invalid("该供应商不支持额度查询"));
        }
        let secret = harness_platform::get_secret(&supplier.credential_ref)?.unwrap_or_default();
        if secret.is_empty() {
            return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
        }
        let value =
            fetch_usage_quota_json(BIGMODEL_USAGE_QUOTA_URL, &secret, supplier.proxy.as_deref())
                .await?;
        map_usage_quota(value)
    }
    pub async fn test_model(&self, id: &str) -> AppResult<()> {
        let config = self
            .list_models()
            .await?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| invalid("模型不存在"))?;
        let mut config = config;
        config.max_output_tokens = Some(512);
        let (secret, extra_headers) = if config.auth == ProviderAuth::ChatGptOAuth {
            oauth_material(&*self.token_store, &config, chrono::Utc::now().timestamp()).await?
        } else {
            let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
            if secret.is_empty()
                && builtin_providers()
                    .iter()
                    .any(|p| p.id == config.provider_id)
            {
                return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
            }
            (secret, Vec::new())
        };
        let answer = HttpModel {
            config,
            secret,
            extra_headers,
            stream_idle_timeout: HttpModel::DEFAULT_STREAM_IDLE_TIMEOUT,
            connection_reuse: true,
        }
        .complete(
            vec![ModelMessage::user("Reply OK.")],
            vec![],
            tokio_util::sync::CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await?;
        if answer.content.trim().is_empty() || !answer.calls.is_empty() {
            return Err(AppError::new(
                "MODEL_PROTOCOL",
                "连接测试未返回有效文字回复",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    async fn service() -> (tempfile::TempDir, ModelService) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("config.db")).await.unwrap();
        (dir, ModelService::new(store))
    }
    /// 内置目录的图片输入能力逐项对照官方文档（2026-11-13 核实）；未核实的模型保持 None（未知）。
    #[test]
    fn builtin_catalog_declares_verified_vision_flags() {
        let providers = builtin_providers();
        let vision = |provider: &str, model: &str| -> Option<bool> {
            providers
                .iter()
                .find(|p| p.id == provider)
                .unwrap()
                .builtin_models
                .iter()
                .find(|m| m.id == model)
                .unwrap()
                .supports_vision
        };
        // DeepSeek 官方 Models & Pricing：Flash 支持 Vision，V4-Pro 不支持。
        assert_eq!(vision("deepseek", "deepseek-flash"), Some(true));
        assert_eq!(vision("deepseek", "deepseek-v4-pro"), Some(false));
        // 智谱官方模型页：5.3/5.2/5-Turbo 输入为文本；5.3-Flash 属 VLM（输入含图像）。
        assert_eq!(vision("bigmodel", "glm-5.3"), Some(false));
        assert_eq!(vision("bigmodel", "glm-5.3-flash"), Some(true));
        assert_eq!(vision("bigmodel", "glm-5.2"), Some(false));
        assert_eq!(vision("bigmodel", "glm-5-turbo"), Some(false));
        // OpenAI 官方模型页：9 个目录模型均带 Image（Input）徽标。
        for model in providers
            .iter()
            .find(|p| p.id == "chatgpt")
            .unwrap()
            .builtin_models
            .iter()
        {
            assert_eq!(model.supports_vision, Some(true), "{}", model.id);
        }
        // 合成条目继承目录元数据。
        let flash = builtin_model_config(
            &builtin_providers()[1],
            &builtin_providers()[1].builtin_models[1],
        )
        .unwrap();
        assert_eq!(flash.supports_vision, Some(true));
    }

    #[test]
    fn builtin_catalog_is_exact_and_protocol_specific() {
        let p = builtin_providers();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].id, "deepseek");
        assert_eq!(p[0].default_model, "deepseek-flash");
        assert_eq!(p[0].auth, ProviderAuth::ApiKey);
        assert_eq!(p[1].id, "bigmodel");
        assert_eq!(p[1].default_model, "glm-5.3");
        assert_eq!(p[1].auth, ProviderAuth::ApiKey);
        assert_eq!(p[2].id, "chatgpt");
        assert_eq!(p[2].auth, ProviderAuth::ChatGptOAuth);
        assert_eq!(p[2].credential_ref, "provider/chatgpt");
        assert_eq!(p[2].endpoints.len(), 1);
        assert_eq!(p[2].endpoints[0].api_mode, ApiMode::Responses);
        assert_eq!(
            p[2].endpoints[0].base_url,
            "https://chatgpt.com/backend-api/codex"
        );
        assert_eq!(p[2].default_model, "gpt-6.1-sol");
        assert_eq!(
            p[0].endpoints[2].base_url,
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            p[1].endpoints[0].base_url,
            "https://open.bigmodel.cn/api/coding/paas/v4"
        );
        assert_eq!(
            p[1].endpoints[1].base_url,
            "https://open.bigmodel.cn/api/v1"
        );
        assert_eq!(
            p[1].endpoints[2].base_url,
            "https://open.bigmodel.cn/api/anthropic"
        );
    }
    #[tokio::test]
    async fn chatgpt_models_require_login_before_build() {
        // 必须注入空凭据存储：系统凭据管理器可能带有开发机真实登录态，会让该测试误判为已登录。
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("config.db")).await.unwrap();
        let service = ModelService::with_token_store(store, Arc::new(MemoryTokenStore::default()));
        let err = service
            .build_model("builtin:chatgpt:gpt-6.1-sol")
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, "MODEL_CREDENTIAL");
        assert!(err.message.contains("ChatGPT"));
        drop(dir);
    }

    /// 内存凭据存储：验证 OAuth 材料构造与 build 链路，不触碰真实凭据管理器。
    #[derive(Default)]
    struct MemoryTokenStore(std::sync::Mutex<std::collections::HashMap<String, String>>);
    impl crate::chatgpt::TokenStore for MemoryTokenStore {
        fn load(&self, reference: &str) -> AppResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(reference).cloned())
        }
        fn save(&self, reference: &str, value: &str) -> AppResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(reference.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, reference: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(reference);
            Ok(())
        }
    }

    #[tokio::test]
    async fn oauth_material_yields_access_token_and_identity_headers() {
        use crate::chatgpt::{TokenBundle, save_bundle};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("config.db")).await.unwrap();
        let memory = std::sync::Arc::new(MemoryTokenStore::default());
        save_bundle(
            &*memory,
            &TokenBundle {
                access_token: "acc-token".into(),
                refresh_token: "rt".into(),
                account_id: "acc-9".into(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
                email: None,
                plan_type: None,
            },
        )
        .unwrap();
        let service = ModelService::with_token_store(store, memory.clone());
        let config = service
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.id == "builtin:chatgpt:gpt-6.1-sol")
            .unwrap();
        assert_eq!(config.auth, ProviderAuth::ChatGptOAuth);
        assert_eq!(config.api_mode, ApiMode::Responses);
        let (secret, headers) =
            oauth_material(memory.as_ref(), &config, chrono::Utc::now().timestamp())
                .await
                .unwrap();
        assert_eq!(secret, "acc-token");
        assert!(headers.contains(&("chatgpt-account-id".to_string(), "acc-9".to_string())));
        assert!(headers.contains(&("originator".to_string(), "codex_cli_rs".to_string())));
        assert!(headers.iter().any(|(name, _)| name == "session-id"));
        // 未过期的 token 不触发刷新（无网络），build 链路走通并指向 ChatGPT 后端。
        let model = service
            .build_model("builtin:chatgpt:gpt-6.1-sol")
            .await
            .unwrap();
        let route = model.route().unwrap();
        assert_eq!(route.base_url, "https://chatgpt.com/backend-api/codex");
        drop(dir);
    }

    #[test]
    fn maps_quota_response_to_remaining_cards() {
        // 样例取自真实接口响应（2026-11 实测）：percentage 为已用口径，展示前反转。
        let value = serde_json::json!({
            "code": 200, "msg": "操作成功", "success": true,
            "data": {
                "level": "pro",
                "limits": [
                    {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 30},
                    {"type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 86, "nextResetTime": 1794130964977i64},
                    {"type": "TIME_LIMIT", "unit": 5, "number": 1, "usage": 1000, "currentValue": 14, "remaining": 986, "percentage": 1, "nextResetTime": 1796092406981i64}
                ]
            }
        });
        let snapshot = map_usage_quota(value).unwrap();
        assert_eq!(snapshot.plan_name.as_deref(), Some("GLM Coding Pro"));
        assert_eq!(snapshot.cards.len(), 3);
        assert_eq!(snapshot.cards[0].kind, UsageCardKind::FiveHour);
        assert_eq!(snapshot.cards[0].remaining_percent, Some(70.0));
        assert_eq!(snapshot.cards[0].reset_at, None);
        assert_eq!(snapshot.cards[1].kind, UsageCardKind::Weekly);
        assert_eq!(snapshot.cards[1].remaining_percent, Some(14.0));
        assert_eq!(snapshot.cards[1].reset_at, Some(1794130964977));
        assert_eq!(snapshot.cards[2].kind, UsageCardKind::Tools);
        assert!((snapshot.cards[2].remaining_percent.unwrap() - 99.0).abs() < 1e-9);
        assert_eq!(snapshot.cards[2].reset_at, Some(1796092406981));
    }
    #[test]
    fn maps_quota_reorders_limits_and_ignores_unknown_entries() {
        // limits 顺序打乱仍按 5小时→每周→工具调用 输出；未知 type/unit 忽略；percentage 缺失回退 remaining/usage。
        let value = serde_json::json!({
            "success": true,
            "data": {
                "level": "GLM Coding Pro",
                "limits": [
                    {"type": "TIME_LIMIT", "unit": 5, "usage": 1000, "remaining": 986},
                    {"type": "WEIRD_LIMIT", "unit": 3, "percentage": 50},
                    {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 86},
                    {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 0}
                ]
            }
        });
        let snapshot = map_usage_quota(value).unwrap();
        assert_eq!(snapshot.plan_name.as_deref(), Some("GLM Coding Pro"));
        let kinds: Vec<UsageCardKind> = snapshot.cards.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                UsageCardKind::FiveHour,
                UsageCardKind::Weekly,
                UsageCardKind::Tools
            ]
        );
        assert_eq!(snapshot.cards[0].remaining_percent, Some(100.0));
        assert_eq!(snapshot.cards[1].remaining_percent, Some(14.0));
        assert!((snapshot.cards[2].remaining_percent.unwrap() - 98.6).abs() < 1e-9);
    }
    #[test]
    fn rejects_quota_error_envelope() {
        let err = map_usage_quota(serde_json::json!({"code":401,"msg":"未授权","success":false}))
            .unwrap_err();
        assert_eq!(err.code, "MODEL_USAGE");
        assert!(err.message.contains("未授权"));
    }
    #[test]
    fn quota_without_level_yields_no_plan_name() {
        let snapshot =
            map_usage_quota(serde_json::json!({"code":200,"success":true,"data":{"limits":[]}}))
                .unwrap();
        assert_eq!(snapshot.plan_name, None);
        assert!(snapshot.cards.is_empty());
    }
    #[test]
    fn maps_chatgpt_usage_response_to_remaining_cards() {
        // 样例按 codex /wham/usage 的 wire 格式构造：used_percent 为已用口径，展示前反转。
        let value = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {"used_percent": 40, "limit_window_seconds": 18000, "reset_after_seconds": 600, "reset_at": 1794130964},
                "secondary_window": {"used_percent": 10, "limit_window_seconds": 604800, "reset_after_seconds": 86400}
            }
        });
        let snapshot = map_chatgpt_usage(value, 1_000).unwrap();
        assert_eq!(snapshot.plan_name.as_deref(), Some("ChatGPT Plus"));
        assert_eq!(snapshot.cards.len(), 2);
        assert_eq!(snapshot.cards[0].kind, UsageCardKind::FiveHour);
        assert_eq!(snapshot.cards[0].remaining_percent, Some(60.0));
        assert_eq!(snapshot.cards[0].reset_at, Some(1_794_130_964_000));
        assert_eq!(snapshot.cards[1].kind, UsageCardKind::Weekly);
        assert_eq!(snapshot.cards[1].remaining_percent, Some(90.0));
        // reset_at 缺失 → now(1000) + reset_after_seconds(86400)，折毫秒。
        assert_eq!(snapshot.cards[1].reset_at, Some(87_400_000));
    }
    #[test]
    fn chatgpt_usage_reset_falls_back_to_reset_after() {
        // reset_at 缺失时按 now + reset_after_seconds 推导（毫秒）。
        let value = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {"used_percent": 0, "limit_window_seconds": 18000, "reset_after_seconds": 600}
            }
        });
        let snapshot = map_chatgpt_usage(value, 1_000).unwrap();
        assert_eq!(snapshot.plan_name.as_deref(), Some("ChatGPT Pro"));
        assert_eq!(snapshot.cards[0].reset_at, Some(1_600_000));
    }
    #[test]
    fn chatgpt_usage_without_windows_yields_no_cards() {
        let snapshot = map_chatgpt_usage(serde_json::json!({"plan_type": "free"}), 0).unwrap();
        assert_eq!(snapshot.plan_name.as_deref(), Some("ChatGPT Free"));
        assert!(snapshot.cards.is_empty());
    }
    #[tokio::test]
    async fn immutable_builtins_and_independent_copy() {
        let (_dir, s) = service().await;
        assert!(
            s.list_models().await.unwrap().iter().all(|m| m.builtin),
            "尚未保存自定义模型时列表只含内置目录条目"
        );
        let mut original = s.list_providers().await.unwrap().remove(0);
        original.name = "changed".into();
        assert!(s.save_provider(original, None).await.is_err());
        assert!(s.remove_provider("deepseek").await.is_err());
        let copy = s.copy_provider("deepseek").await.unwrap();
        assert!(!copy.builtin);
        assert_ne!(copy.credential_ref, "provider/deepseek");
        assert_eq!(copy.endpoints, builtin_providers()[0].endpoints);
    }
    #[tokio::test]
    async fn authoritative_resolution_and_supplier_credential_isolation() {
        let (_dir, s) = service().await;
        let mut supplier = s.copy_provider("deepseek").await.unwrap();
        supplier.credential_ref = "provider/bigmodel".into();
        supplier.proxy = Some("http://localhost:1234".into());
        let supplier = s.save_provider(supplier, None).await.unwrap();
        assert_ne!(supplier.credential_ref, "provider/bigmodel");
        let config = ModelConfig {
            compatibility: ProviderCompatibility::Standard,
            id: String::new(),
            name: "local".into(),
            provider_id: supplier.id.clone(),
            api_mode: ApiMode::Responses,
            max_output_tokens: None,
            base_url: "http://evil.test".into(),
            model: "m".into(),
            credential_ref: "provider/bigmodel".into(),
            proxy: None,
            ..ModelConfig::default()
        };
        let saved = s.save_model(config, None).await.unwrap();
        assert_eq!(saved.base_url, "https://api.deepseek.com");
        assert_eq!(saved.proxy, supplier.proxy);
        assert_eq!(saved.credential_ref, supplier.credential_ref);
        assert!(s.remove_provider(&supplier.id).await.is_err());
        s.remove_model(&saved.id).await.unwrap();
        s.remove_provider(&supplier.id).await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_model_saves_preserve_all_updates() {
        let (_dir, s) = service().await;
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..12 {
            let s = s.clone();
            tasks.spawn(async move {
                s.save_model(
                    ModelConfig {
                        compatibility: ProviderCompatibility::Standard,
                        id: format!("m{i}"),
                        name: "m".into(),
                        provider_id: "deepseek".into(),
                        api_mode: ApiMode::ChatCompletions,
                        max_output_tokens: None,
                        base_url: String::new(),
                        model: "m".into(),
                        credential_ref: String::new(),
                        proxy: None,
                        ..ModelConfig::default()
                    },
                    None,
                )
                .await
                .unwrap();
            });
        }
        while let Some(r) = tasks.join_next().await {
            r.unwrap();
        }
        assert_eq!(
            s.list_models()
                .await
                .unwrap()
                .iter()
                .filter(|m| !m.builtin)
                .count(),
            12
        );
    }
    #[tokio::test]
    async fn refuses_builtin_endpoint_override() {
        let (_dir, s) = service().await;
        let config = ModelConfig {
            compatibility: ProviderCompatibility::Standard,
            id: String::new(),
            name: "m".into(),
            provider_id: "deepseek".into(),
            api_mode: ApiMode::Responses,
            max_output_tokens: None,
            base_url: "https://evil.test".into(),
            model: "m".into(),
            credential_ref: String::new(),
            proxy: None,
            ..ModelConfig::default()
        };
        assert!(s.save_model(config, None).await.is_err());
        assert!(
            s.list_models().await.unwrap().iter().all(|m| m.builtin),
            "被拒绝的端点覆盖不得留下自定义模型"
        );
    }
    #[tokio::test]
    async fn referenced_protocol_cannot_be_removed() {
        let (_dir, s) = service().await;
        let mut supplier = s.copy_provider("deepseek").await.unwrap();
        s.save_model(
            ModelConfig {
                compatibility: ProviderCompatibility::Standard,
                id: String::new(),
                name: "m".into(),
                provider_id: supplier.id.clone(),
                api_mode: ApiMode::Responses,
                max_output_tokens: None,
                base_url: String::new(),
                model: "m".into(),
                credential_ref: String::new(),
                proxy: None,
                ..ModelConfig::default()
            },
            None,
        )
        .await
        .unwrap();
        supplier
            .endpoints
            .retain(|e| e.api_mode != ApiMode::Responses);
        assert!(s.save_provider(supplier, None).await.is_err());
        assert_eq!(
            s.list_models()
                .await
                .unwrap()
                .iter()
                .filter(|m| !m.builtin)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn connection_test_requires_visible_text() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, s) = service().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut input = vec![0; 8192];
            assert!(stream.read(&mut input).await.unwrap() > 0);
            let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"thinking only\"}}]}\n\ndata: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let mut supplier = s.copy_provider("deepseek").await.unwrap();
        supplier.endpoints = vec![ProtocolEndpoint {
            api_mode: ApiMode::ChatCompletions,
            base_url: format!("http://{address}"),
        }];
        let supplier = s.save_provider(supplier, None).await.unwrap();
        let model = s
            .save_model(
                ModelConfig {
                    compatibility: ProviderCompatibility::Standard,
                    id: String::new(),
                    name: "test".into(),
                    provider_id: supplier.id,
                    api_mode: ApiMode::ChatCompletions,
                    max_output_tokens: None,
                    base_url: String::new(),
                    model: "test".into(),
                    credential_ref: String::new(),
                    proxy: None,
                    ..ModelConfig::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            s.test_model(&model.id).await.unwrap_err().code,
            "MODEL_PROTOCOL"
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn copied_supplier_preserves_compatibility_and_models_use_supplier_profile() {
        let (_dir, s) = service().await;
        for (id, expected) in [
            ("deepseek", ProviderCompatibility::DeepSeek),
            ("bigmodel", ProviderCompatibility::BigModel),
        ] {
            let copy = s.copy_provider(id).await.unwrap();
            assert_eq!(copy.compatibility, expected);
            assert_ne!(copy.credential_ref, format!("provider/{id}"));
            let model = s
                .save_model(
                    ModelConfig {
                        compatibility: ProviderCompatibility::Standard,
                        id: String::new(),
                        name: "copy".into(),
                        provider_id: copy.id,
                        api_mode: ApiMode::Responses,
                        max_output_tokens: None,
                        base_url: String::new(),
                        model: "m".into(),
                        credential_ref: String::new(),
                        proxy: None,
                        ..ModelConfig::default()
                    },
                    None,
                )
                .await
                .unwrap();
            assert_eq!(model.compatibility, expected);
        }
        let mut builtin = s.list_providers().await.unwrap().remove(0);
        builtin.compatibility = ProviderCompatibility::Standard;
        assert!(s.save_provider(builtin, None).await.is_err());
    }
    #[tokio::test]
    async fn connection_test_budget_allows_short_reasoning_responses() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut part = [0; 4096];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&part[..n]);
                if let Some(p) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break p + 4;
                }
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let size = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while bytes.len() < end + size {
                let mut part = [0; 4096];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&part[..n]);
            }
            let request: serde_json::Value =
                serde_json::from_slice(&bytes[end..end + size]).unwrap();
            let event = if request["max_output_tokens"].as_u64().unwrap_or(0) < 512 {
                json!({"type":"response.incomplete","response":{"status":"incomplete"}})
            } else {
                json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}]}})
            };
            let body = format!("data: {event}\n\n");
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let (_dir, service) = service().await;
        let mut supplier = service.copy_provider("bigmodel").await.unwrap();
        supplier.endpoints = vec![ProtocolEndpoint {
            api_mode: ApiMode::Responses,
            base_url: url,
        }];
        let supplier = service.save_provider(supplier, None).await.unwrap();
        let model:ModelConfig=serde_json::from_value(json!({"id":"","name":"budget","providerId":supplier.id,"apiMode":"responses","baseUrl":"","model":"fixture","credentialRef":"","proxy":null})).unwrap();
        let model = service.save_model(model, None).await.unwrap();
        let result = service.test_model(&model.id).await;
        task.await.unwrap();
        assert!(
            result.is_ok(),
            "connection test must leave enough budget for a short thinking response: {result:?}"
        );
    }

    #[test]
    fn builtin_catalog_declares_models_with_efforts() {
        let p = builtin_providers();
        let deepseek = p.iter().find(|provider| provider.id == "deepseek").unwrap();
        let bigmodel = p.iter().find(|provider| provider.id == "bigmodel").unwrap();
        let deepseek_ids: Vec<_> = deepseek
            .builtin_models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(deepseek_ids, ["deepseek-flash", "deepseek-v4-pro"]);
        let bigmodel_ids: Vec<_> = bigmodel
            .builtin_models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            bigmodel_ids,
            ["glm-5.3", "glm-5.3-flash", "glm-5.2", "glm-5-turbo"]
        );
        assert_eq!(bigmodel.builtin_models[0].efforts, ["low", "high", "max"]);
        assert_eq!(deepseek.builtin_models[0].efforts, ["disabled", "enabled"]);
        // 窗口预填值锁定：glm-5.2/5.3/5.3-flash 为 1M，5-turbo 沿用 200k。
        assert_eq!(
            bigmodel
                .builtin_models
                .iter()
                .map(|m| m.context_window)
                .collect::<Vec<_>>(),
            [
                Some(1_000_000),
                Some(1_000_000),
                Some(1_000_000),
                Some(200_000)
            ]
        );
        // 输出上限预填值锁定：智谱系 128K（官方最大输出），deepseek 384K，chatgpt 后端不消费该字段。
        assert_eq!(
            bigmodel
                .builtin_models
                .iter()
                .map(|m| m.max_output_tokens)
                .collect::<Vec<_>>(),
            [Some(131_072); 4]
        );
        assert_eq!(
            deepseek
                .builtin_models
                .iter()
                .map(|m| m.max_output_tokens)
                .collect::<Vec<_>>(),
            [Some(393_216); 2]
        );
        let chatgpt = p.iter().find(|p| p.id == "chatgpt").unwrap();
        assert!(
            chatgpt
                .builtin_models
                .iter()
                .all(|m| m.max_output_tokens.is_none())
        );
        // 内置模型走供应商声明的协议端点（既有目录为 Anthropic，chatgpt 为 Responses）。
        for provider in &p {
            let expected_endpoint = provider
                .endpoints
                .iter()
                .find(|e| e.api_mode == ApiMode::Anthropic)
                .or_else(|| provider.endpoints.first())
                .unwrap();
            for model in &provider.builtin_models {
                let config = builtin_model_config(provider, model).unwrap();
                assert_eq!(config.api_mode, expected_endpoint.api_mode);
                assert_eq!(config.base_url, expected_endpoint.base_url);
                assert_eq!(config.auth, provider.auth);
                assert!(config.builtin);
                assert_eq!(config.model, model.id);
                assert_eq!(config.provider_id, provider.id);
                // 合成的可路由配置必须带上目录声明的输出上限。
                assert_eq!(config.max_output_tokens, model.max_output_tokens);
            }
        }
    }

    #[test]
    fn chatgpt_terra_uses_codex_default_context_window() {
        let terra = builtin_providers()
            .into_iter()
            .find(|provider| provider.id == "chatgpt")
            .unwrap()
            .builtin_models
            .into_iter()
            .find(|model| model.id == "gpt-6.1-terra")
            .unwrap();

        assert_eq!(terra.context_window, Some(500_000));
    }

    #[tokio::test]
    async fn builtin_models_are_synthesized_and_can_be_disabled() {
        let (_dir, s) = service().await;
        let flash = s
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.id == "builtin:bigmodel:glm-5.3-flash")
            .unwrap();
        assert_eq!(flash.base_url, "https://open.bigmodel.cn/api/anthropic");
        assert_eq!(flash.compatibility, ProviderCompatibility::BigModel);
        assert_eq!(flash.credential_ref, "provider/bigmodel");
        assert!(flash.enabled);
        assert_eq!(flash.efforts, ["low", "high", "max"]);
        // 内置模型凭据缺失时不可执行（纯函数校验，避免测试依赖机器凭据状态）。
        require_credential(&flash, "").unwrap_err();
        require_credential(&flash, "k").unwrap();
        // 禁用后条目标记停用，构建在凭据环节之前被拦截。
        s.set_builtin_model_enabled("bigmodel", "glm-5.3-flash", false)
            .await
            .unwrap();
        let flash = s
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.id == "builtin:bigmodel:glm-5.3-flash")
            .unwrap();
        assert!(!flash.enabled);
        assert_eq!(
            s.build_model(&flash.id).await.err().unwrap().code,
            "MODEL_DISABLED"
        );
        // 重新启用恢复。
        s.set_builtin_model_enabled("bigmodel", "glm-5.3-flash", true)
            .await
            .unwrap();
        let flash = s
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.id == "builtin:bigmodel:glm-5.3-flash")
            .unwrap();
        assert!(flash.enabled);
    }

    #[tokio::test]
    async fn disabling_provider_hides_all_its_models() {
        let (_dir, s) = service().await;
        let mut deepseek = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "deepseek")
            .unwrap();
        deepseek.enabled = false;
        s.save_provider(deepseek, None).await.unwrap();
        assert!(
            !s.list_providers()
                .await
                .unwrap()
                .iter()
                .find(|p| p.id == "deepseek")
                .unwrap()
                .enabled
        );
        // 供应商停用后其模型不可构建。
        assert_eq!(
            s.build_model("builtin:deepseek:deepseek-flash")
                .await
                .err()
                .unwrap()
                .code,
            "MODEL_DISABLED"
        );
        // 重新启用后恢复。
        let mut deepseek = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "deepseek")
            .unwrap();
        deepseek.enabled = true;
        s.save_provider(deepseek, None).await.unwrap();
        assert!(
            s.list_providers()
                .await
                .unwrap()
                .iter()
                .find(|p| p.id == "deepseek")
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn builtin_provider_saves_only_allow_enable_and_model_toggles() {
        let (_dir, s) = service().await;
        let mut bigmodel = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "bigmodel")
            .unwrap();
        bigmodel.name = "改名".into();
        assert!(s.save_provider(bigmodel, None).await.is_err());
        let mut bigmodel = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "bigmodel")
            .unwrap();
        bigmodel.endpoints.clear();
        assert!(s.save_provider(bigmodel, None).await.is_err());
        // 启停、代理与内置模型禁用清单是内置供应商仅有的可改项。
        let mut bigmodel = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "bigmodel")
            .unwrap();
        bigmodel.enabled = false;
        bigmodel.disabled_models = vec!["glm-5-turbo".into()];
        bigmodel.proxy = Some("http://127.0.0.1:7890".into());
        let saved = s.save_provider(bigmodel, None).await.unwrap();
        assert!(!saved.enabled);
        assert_eq!(saved.disabled_models, ["glm-5-turbo"]);
        assert_eq!(saved.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        let merged = s
            .list_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "bigmodel")
            .unwrap();
        assert!(!merged.enabled);
        assert_eq!(merged.disabled_models, ["glm-5-turbo"]);
        assert_eq!(merged.endpoints.len(), 3, "端点保持目录值");
    }

    #[tokio::test]
    async fn save_model_validates_efforts_and_refuses_builtin_entries() {
        let (_dir, s) = service().await;
        let unsupported: ModelConfig = serde_json::from_value(json!({
            "id":"","name":"m","providerId":"deepseek","apiMode":"chatCompletions",
            "baseUrl":"","model":"m","credentialRef":"","proxy":null,
            "efforts":["low","bogus"],"effort":"low"
        }))
        .unwrap();
        assert!(s.save_model(unsupported, None).await.is_err());
        let mismatch: ModelConfig = serde_json::from_value(json!({
            "id":"","name":"m","providerId":"deepseek","apiMode":"chatCompletions",
            "baseUrl":"","model":"m","credentialRef":"","proxy":null,
            "efforts":["low","high"],"effort":"max"
        }))
        .unwrap();
        assert!(s.save_model(mismatch, None).await.is_err());
        let builtin: ModelConfig = serde_json::from_value(json!({
            "id":"builtin:deepseek:deepseek-flash","name":"DeepSeek Flash","providerId":"deepseek",
            "apiMode":"anthropic","baseUrl":"","model":"deepseek-flash","credentialRef":"","proxy":null,
            "builtin":true
        }))
        .unwrap();
        assert!(s.save_model(builtin, None).await.is_err());
        let valid: ModelConfig = serde_json::from_value(json!({
            "id":"","name":"m","providerId":"deepseek","apiMode":"chatCompletions",
            "baseUrl":"","model":"m","credentialRef":"","proxy":null,
            "efforts":["low","high"],"effort":"high"
        }))
        .unwrap();
        let saved = s.save_model(valid, None).await.unwrap();
        assert_eq!(saved.effort.as_deref(), Some("high"));
        assert_eq!(saved.efforts, ["low", "high"]);
    }

    #[tokio::test]
    async fn build_model_with_validates_effort_before_credentials() {
        let (_dir, s) = service().await;
        // 不在支持档位内的 effort 在读凭据之前就被拒绝。
        assert_eq!(
            s.build_model_with("builtin:bigmodel:glm-5.3", Some("bogus".into()))
                .await
                .err()
                .unwrap()
                .code,
            "MODEL_EFFORT"
        );
        // 查找成功且档位合法时即通过 effort 校验（后续凭据环节由 require_credential 单测覆盖）。
        let effort = apply_effort(
            s.list_models()
                .await
                .unwrap()
                .into_iter()
                .find(|m| m.id == "builtin:bigmodel:glm-5.3")
                .unwrap(),
            Some("low".into()),
        )
        .unwrap();
        assert_eq!(effort.effort.as_deref(), Some("low"));
        // 模型不存在仍报 MODEL_NOT_FOUND。
        assert_eq!(
            s.build_model_with("builtin:bigmodel:nope", None)
                .await
                .err()
                .unwrap()
                .code,
            "MODEL_NOT_FOUND"
        );
        // 停用模型在凭据环节之前被拒绝。
        s.set_builtin_model_enabled("bigmodel", "glm-5.3", false)
            .await
            .unwrap();
        assert_eq!(
            s.build_model_with("builtin:bigmodel:glm-5.3", None)
                .await
                .err()
                .unwrap()
                .code,
            "MODEL_DISABLED"
        );
    }

    #[test]
    fn apply_effort_overrides_and_validates() {
        let config = builtin_model_config(
            &builtin_providers()[1],
            &builtin_providers()[1].builtin_models[0],
        )
        .unwrap();
        assert_eq!(config.effort, None);
        let config = apply_effort(config, Some("low".into())).unwrap();
        assert_eq!(config.effort.as_deref(), Some("low"));
        let error = apply_effort(config.clone(), Some("minimal".into())).unwrap_err();
        assert_eq!(error.code, "MODEL_EFFORT");
        // 不支持档位的模型只能跟随默认。
        let mut plain = config.clone();
        plain.efforts.clear();
        assert!(apply_effort(plain, Some("low".into())).is_err());
        // None 表示清除覆盖，跟随供应商默认。
        let cleared = apply_effort(config.clone(), None).unwrap();
        assert_eq!(cleared.effort, None);
    }

    #[tokio::test]
    async fn fetch_available_models_requires_credential_and_parses_ids() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, s) = service().await;
        // 未配置密钥的供应商副本 → MODEL_CREDENTIAL（不用内置 ID，避免读到机器真实凭据）。
        let copy = s.copy_provider("bigmodel").await.unwrap();
        assert_eq!(
            s.fetch_available_models(&copy.id, ApiMode::Anthropic)
                .await
                .unwrap_err()
                .code,
            "MODEL_CREDENTIAL"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut input = vec![0; 8192];
            let n = stream.read(&mut input).await.unwrap();
            let request = String::from_utf8_lossy(&input[..n]).to_string();
            assert!(
                request.contains("GET /v1/models"),
                "anthropic 模式应请求 /v1/models：{request}"
            );
            assert!(request.contains("x-api-key"));
            let body = r#"{"data":[{"id":"z-model"},{"id":"a-model"},{"id":"a-model"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        // 核心解析函数直接测，避免在测试里写入真实系统凭据。
        let ids = fetch_model_ids(
            &format!("http://{address}"),
            ApiMode::Anthropic,
            "k-test",
            None,
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(ids, ["a-model", "z-model"]);
        // 不存在的供应商报错。
        assert_eq!(
            s.fetch_available_models("nope", ApiMode::Anthropic)
                .await
                .unwrap_err()
                .code,
            "MODEL_CONFIG"
        );
    }
}
