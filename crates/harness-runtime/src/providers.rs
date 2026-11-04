//! Shared supplier configuration and authoritative route resolution for every carrier.
use crate::model::{HttpModel, ModelMessage, ModelProvider};
use harness_contracts::*;
use harness_storage::Store;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn builtin_providers() -> Vec<ProviderInfo> {
    [
        (
            "deepseek",
            "DeepSeek",
            "deepseek-flash",
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
            [
                "https://open.bigmodel.cn/api/coding/paas/v4",
                "https://open.bigmodel.cn/api/v1",
                "https://open.bigmodel.cn/api/anthropic",
            ],
        ),
    ]
    .into_iter()
    .map(|(id, name, model, urls)| ProviderInfo {
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
    })
    .collect()
}
fn invalid(message: &str) -> AppError {
    AppError::new("MODEL_CONFIG", message)
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
                builtin.proxy = saved.proxy; // Built-in catalog identity and URLs are immutable.
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
    pub async fn list_models(&self) -> AppResult<Vec<ModelConfig>> {
        let _guard = self.gate.lock().await;
        let suppliers = self.providers().await?;
        self.read::<ModelConfig>("models")
            .await?
            .into_iter()
            .map(|m| Self::resolve(m, &suppliers))
            .collect()
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
        let config = self
            .list_models()
            .await?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| AppError::new("MODEL_NOT_FOUND", "模型不存在"))?;
        let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
        if secret.is_empty()
            && builtin_providers()
                .iter()
                .any(|p| p.id == config.provider_id)
        {
            return Err(AppError::new("MODEL_CREDENTIAL", "请先配置供应商密钥"));
        }
        Ok(Arc::new(HttpModel { config, secret }))
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
    #[tokio::test]
    async fn immutable_builtins_and_independent_copy() {
        let (_dir, s) = service().await;
        assert!(s.list_models().await.unwrap().is_empty());
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
        assert_eq!(s.list_models().await.unwrap().len(), 12);
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
        };
        assert!(s.save_model(config, None).await.is_err());
        assert!(s.list_models().await.unwrap().is_empty());
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
            },
            None,
        )
        .await
        .unwrap();
        supplier
            .endpoints
            .retain(|e| e.api_mode != ApiMode::Responses);
        assert!(s.save_provider(supplier, None).await.is_err());
        assert_eq!(s.list_models().await.unwrap().len(), 1);
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
}
