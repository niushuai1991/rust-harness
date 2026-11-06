//! Shared supplier configuration and authoritative route resolution for every carrier.
use crate::model::{HttpModel, ModelMessage, ModelProvider, transport_error};
use harness_contracts::*;
use harness_storage::Store;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 内置供应商目录（deepseek、bigmodel）及其只读模型清单，与 zcode 内置目录对齐。
/// 目录身份在升级间保持稳定；用户只能通过个人覆盖层启用/禁用，不能改写。
pub fn builtin_providers() -> Vec<ProviderInfo> {
    let deepseek_models: [(&str, &str, &[&str]); 2] = [
        ("deepseek-flash", "DeepSeek Flash", &["disabled", "enabled"]),
        (
            "deepseek-v4-pro",
            "DeepSeek V4 Pro",
            &["disabled", "enabled"],
        ),
    ];
    let bigmodel_models: [(&str, &str, &[&str]); 4] = [
        ("glm-5.3", "GLM-5.3", &["low", "high", "max"]),
        ("glm-5.3-flash", "GLM-5.3 Flash", &["low", "high", "max"]),
        ("glm-5.2", "GLM-5.2", &["disabled", "high", "max"]),
        ("glm-5-turbo", "GLM-5 Turbo", &["disabled", "enabled"]),
    ];
    [
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
            .map(|(id, name, efforts)| BuiltinModel {
                id: (*id).into(),
                name: (*name).into(),
                efforts: efforts.iter().map(|e| (*e).into()).collect(),
            })
            .collect(),
        disabled_models: Vec::new(),
    })
    .collect()
}

/// 由内置目录条目合成的可路由模型配置；api_mode 固定为供应商的 Anthropic 端点。
fn builtin_model_config(provider: &ProviderInfo, model: &BuiltinModel) -> Option<ModelConfig> {
    let endpoint = provider
        .endpoints
        .iter()
        .find(|e| e.api_mode == ApiMode::Anthropic)?;
    Some(ModelConfig {
        provider_id: provider.id.clone(),
        id: format!("builtin:{}:{}", provider.id, model.id),
        name: model.name.clone(),
        base_url: endpoint.base_url.clone(),
        model: model.id.clone(),
        credential_ref: provider.credential_ref.clone(),
        proxy: provider.proxy.clone(),
        compatibility: provider.compatibility,
        api_mode: ApiMode::Anthropic,
        max_output_tokens: None,
        efforts: model.efforts.clone(),
        effort: None,
        enabled: !provider.disabled_models.contains(&model.id),
        builtin: true,
    })
}

/// 内置供应商的模型必须配置密钥才能执行；自定义供应商允许空密钥（请求时由端点拒绝）。
fn require_credential(config: &ModelConfig, secret: &str) -> AppResult<()> {
    if secret.is_empty()
        && builtin_providers()
            .iter()
            .any(|p| p.id == config.provider_id)
    {
        return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
    }
    Ok(())
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
        // 按固定顺序匹配三张卡（与 zcode 的 type/unit 约定一致），未知条目忽略。
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
}
impl ModelService {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            gate: Arc::new(Mutex::new(())),
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
        let mut config = self
            .list_models()
            .await?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| AppError::new("MODEL_NOT_FOUND", "模型不存在"))?;
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
        let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
        require_credential(&config, &secret)?;
        Ok(Arc::new(HttpModel { config, secret }))
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
    /// 拉取供应商套餐与剩余额度（当前仅内置智谱 Coding Plan 支持）；快照即查即返，不落库。
    pub async fn fetch_usage(&self, provider_id: &str) -> AppResult<ProviderUsageSnapshot> {
        let _guard = self.gate.lock().await;
        let supplier = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| invalid("供应商不存在"))?;
        if !supplier.builtin || supplier.compatibility != ProviderCompatibility::BigModel {
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
        let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
        if secret.is_empty()
            && builtin_providers()
                .iter()
                .any(|p| p.id == config.provider_id)
        {
            return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
        }
        let mut config = config;
        config.max_output_tokens = Some(512);
        let answer = HttpModel { config, secret }
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
    #[test]
    fn builtin_catalog_is_exact_and_protocol_specific() {
        let p = builtin_providers();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].id, "deepseek");
        assert_eq!(p[0].default_model, "deepseek-flash");
        assert_eq!(p[1].id, "bigmodel");
        assert_eq!(p[1].default_model, "glm-5.3");
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
        // 内置模型全部走 Anthropic 端点，与 zcode 模板保持一致。
        for provider in &p {
            for model in &provider.builtin_models {
                let config = builtin_model_config(provider, model).unwrap();
                assert_eq!(config.api_mode, ApiMode::Anthropic);
                assert!(config.builtin);
                assert_eq!(config.model, model.id);
                assert_eq!(config.provider_id, provider.id);
            }
        }
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
