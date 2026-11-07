//! 配置预设：命名的会话能力配置包（docs/specs/config-presets-spec.md）。
//! 分层解析在各宿主端进行：显式参数 > 预设字段 > 宿主缺省回落。

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use harness_contracts::{
    AppError, AppResult, KNOWN_APPROVALS, ModelConfig, PresetConfig, PresetInfo, ProviderInfo,
};
use harness_storage::Store;

use crate::providers::ModelService;

/// 合成内置预设：无任何覆盖，选中即现状行为；只读，不落库。
pub const BUILTIN_PRESET_ID: &str = "builtin:default";

const PRESETS_KEY: &str = "presets";

pub fn builtin_preset() -> PresetConfig {
    PresetConfig {
        id: BUILTIN_PRESET_ID.into(),
        name: "默认（无覆盖）".into(),
        description: Some("不覆盖模型、档位与审批策略，行为与未选预设一致".into()),
        model_id: None,
        effort: None,
        approval: None,
    }
}

/// 预设解析结果；字段为 None 表示回落宿主缺省（`cli.default_model` / TTY 规则 / 模型默认档位）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPreset {
    pub model_id: Option<String>,
    pub effort: Option<String>,
    pub approval: Option<String>,
}

#[derive(Clone)]
pub struct PresetService {
    store: Store,
    models: ModelService,
    gate: Arc<Mutex<()>>,
}

impl PresetService {
    pub fn new(store: Store, models: ModelService) -> Self {
        Self {
            store,
            models,
            gate: Arc::new(Mutex::new(())),
        }
    }

    async fn read(&self) -> AppResult<Vec<PresetConfig>> {
        serde_json::from_value(self.store.get(PRESETS_KEY).await?.unwrap_or(json!([])))
            .map_err(|e| AppError::new("PRESET_CONFIG", e.to_string()))
    }

    /// 全量预设视图：合成内置置顶，其后用户预设逐条附 broken 标记。
    pub async fn list(&self) -> AppResult<Vec<PresetInfo>> {
        let _guard = self.gate.lock().await;
        let models = self.models.list_models().await?;
        let providers = self.models.list_providers().await?;
        let mut out = vec![PresetInfo {
            preset: builtin_preset(),
            broken: None,
        }];
        for preset in self.read().await? {
            let broken = broken_reason(&preset, &models, &providers);
            out.push(PresetInfo { preset, broken });
        }
        Ok(out)
    }

    /// 保存（upsert）；校验只查目录，不做凭据读取与网络。
    pub async fn save(&self, mut config: PresetConfig) -> AppResult<PresetConfig> {
        let _guard = self.gate.lock().await;
        if config.id.trim().is_empty() {
            return Err(invalid("预设 ID 不能为空"));
        }
        if config.id == BUILTIN_PRESET_ID {
            return Err(AppError::new("PRESET_BUILTIN", "内置预设不可修改"));
        }
        if config.name.trim().is_empty() {
            config.name = config.id.clone();
        }
        if let Some(model_id) = &config.model_id {
            let model = self
                .models
                .list_models()
                .await?
                .into_iter()
                .find(|m| &m.id == model_id)
                .ok_or_else(|| invalid(&format!("模型 {model_id} 不存在")))?;
            if let Some(effort) = &config.effort
                && !model.efforts.contains(effort)
            {
                return Err(invalid(&format!(
                    "推理档位 {effort} 不在模型 {model_id} 支持的档位列表内"
                )));
            }
        } else if config.effort.is_some() {
            return Err(invalid("预设未指定模型时不能单独设置档位"));
        }
        if let Some(approval) = &config.approval
            && !KNOWN_APPROVALS.contains(&approval.as_str())
        {
            return Err(invalid(&format!("审批策略 {approval} 不受支持")));
        }
        let mut all = self.read().await?;
        all.retain(|p| p.id != config.id);
        all.push(config.clone());
        self.store.set(PRESETS_KEY, &json!(all)).await?;
        Ok(config)
    }

    /// 删除用户预设；内置拒绝，不存在时幂等。
    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let _guard = self.gate.lock().await;
        if id == BUILTIN_PRESET_ID {
            return Err(AppError::new("PRESET_BUILTIN", "内置预设不可删除"));
        }
        let mut all = self.read().await?;
        all.retain(|p| p.id != id);
        self.store.set(PRESETS_KEY, &json!(all)).await
    }

    /// 解析预设；未知 id 报 `PRESET_NOT_FOUND` 并列出可用项。`builtin:default` 返回全空。
    pub async fn resolve(&self, preset_id: &str) -> AppResult<ResolvedPreset> {
        if preset_id == BUILTIN_PRESET_ID {
            return Ok(ResolvedPreset::default());
        }
        let all = self.read().await?;
        let preset = all.iter().find(|p| p.id == preset_id).ok_or_else(|| {
            let mut ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
            ids.push(BUILTIN_PRESET_ID);
            AppError::new(
                "PRESET_NOT_FOUND",
                format!("预设 {preset_id} 不存在，可用：{}", ids.join(", ")),
            )
        })?;
        Ok(ResolvedPreset {
            model_id: preset.model_id.clone(),
            effort: preset.effort.clone(),
            approval: preset.approval.clone(),
        })
    }
}

/// broken 计算：模型缺失/停用、供应商停用、effort 不在支持列表。
fn broken_reason(
    preset: &PresetConfig,
    models: &[ModelConfig],
    providers: &[ProviderInfo],
) -> Option<String> {
    let model_id = preset.model_id.as_deref()?;
    let Some(model) = models.iter().find(|m| m.id == model_id) else {
        return Some(format!("模型 {model_id} 不存在"));
    };
    if !model.enabled {
        return Some(format!("模型 {model_id} 已停用"));
    }
    if providers
        .iter()
        .find(|p| p.id == model.provider_id)
        .is_some_and(|p| !p.enabled)
    {
        return Some(format!("供应商 {} 已停用", model.provider_id));
    }
    if let Some(effort) = &preset.effort
        && !model.efforts.contains(effort)
    {
        return Some(format!(
            "推理档位 {effort} 不在模型 {model_id} 支持的档位列表内"
        ));
    }
    None
}

fn invalid(message: &str) -> AppError {
    AppError::new("PRESET_CONFIG", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_contracts::{ApiMode, ModelConfig};

    fn preset(
        id: &str,
        model: Option<&str>,
        effort: Option<&str>,
        approval: Option<&str>,
    ) -> PresetConfig {
        PresetConfig {
            id: id.into(),
            name: id.into(),
            description: None,
            model_id: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            approval: approval.map(str::to_owned),
        }
    }

    async fn service() -> (tempfile::TempDir, PresetService) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("config.db")).await.unwrap();
        let models = ModelService::new(store.clone());
        (dir, PresetService::new(store, models))
    }

    #[tokio::test]
    async fn builtin_default_is_synthetic_and_readonly() {
        let (_dir, s) = service().await;
        let list = s.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].preset.id, BUILTIN_PRESET_ID);
        assert_eq!(list[0].broken, None);
        assert_eq!(list[0].preset.model_id, None);
        assert_eq!(
            s.save(builtin_preset()).await.unwrap_err().code,
            "PRESET_BUILTIN"
        );
        assert_eq!(
            s.remove(BUILTIN_PRESET_ID).await.unwrap_err().code,
            "PRESET_BUILTIN"
        );
    }

    #[tokio::test]
    async fn save_upserts_and_resolves() {
        let (_dir, s) = service().await;
        s.save(preset(
            "refactor",
            Some("builtin:bigmodel:glm-5.3"),
            Some("high"),
            Some("auto"),
        ))
        .await
        .unwrap();
        s.save(preset(
            "refactor",
            Some("builtin:bigmodel:glm-5.3"),
            Some("high"),
            Some("auto"),
        ))
        .await
        .unwrap();
        let list = s.list().await.unwrap();
        assert_eq!(list.len(), 2, "内置 + 一条用户预设（upsert 不重复）");
        assert_eq!(list[1].broken, None, "目录内模型与档位均有效");

        let resolved = s.resolve("refactor").await.unwrap();
        assert_eq!(
            resolved.model_id.as_deref(),
            Some("builtin:bigmodel:glm-5.3")
        );
        assert_eq!(resolved.effort.as_deref(), Some("high"));
        assert_eq!(resolved.approval.as_deref(), Some("auto"));
        assert_eq!(
            s.resolve(BUILTIN_PRESET_ID).await.unwrap(),
            ResolvedPreset::default()
        );
    }

    #[tokio::test]
    async fn save_validation_matrix() {
        let (_dir, s) = service().await;
        assert_eq!(
            s.save(preset("a", Some("ghost-model"), None, None))
                .await
                .unwrap_err()
                .code,
            "PRESET_CONFIG",
            "模型必须存在于目录"
        );
        assert!(
            s.save(preset(
                "b",
                Some("builtin:bigmodel:glm-5.3"),
                Some("lowx"),
                None
            ))
            .await
            .is_err(),
            "档位必须在模型支持列表内"
        );
        assert!(
            s.save(preset("c", None, None, Some("yolo"))).await.is_err(),
            "审批策略必须为 ask/auto/never"
        );
        assert!(
            s.save(preset("d", None, Some("high"), None)).await.is_err(),
            "未指定模型时不能单独设置档位"
        );
        assert!(
            s.save(preset("", None, None, None)).await.is_err(),
            "ID 不能为空"
        );
        let unnamed = PresetConfig {
            name: String::new(),
            ..preset("e", Some("builtin:bigmodel:glm-5.3"), None, None)
        };
        assert_eq!(s.save(unnamed).await.unwrap().name, "e", "名称缺省取 id");
    }

    #[tokio::test]
    async fn remove_is_idempotent_for_user_presets() {
        let (_dir, s) = service().await;
        s.remove("ghost").await.unwrap();
        s.save(preset("p1", None, None, Some("never")))
            .await
            .unwrap();
        s.remove("p1").await.unwrap();
        assert_eq!(s.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn resolve_unknown_lists_available() {
        let (_dir, s) = service().await;
        let err = s.resolve("nope").await.unwrap_err();
        assert_eq!(err.code, "PRESET_NOT_FOUND");
        assert!(
            err.message.contains("builtin:default"),
            "错误信息列出可用预设"
        );
    }

    #[tokio::test]
    async fn broken_marks_disabled_and_missing_models() {
        let (_dir, s) = service().await;
        s.save(preset(
            "p",
            Some("builtin:bigmodel:glm-5.3"),
            Some("high"),
            None,
        ))
        .await
        .unwrap();
        s.models
            .set_builtin_model_enabled("bigmodel", "glm-5.3", false)
            .await
            .unwrap();
        assert!(
            s.list().await.unwrap()[1]
                .broken
                .as_deref()
                .unwrap()
                .contains("已停用"),
            "模型停用后预设标记 broken"
        );
        s.models
            .set_builtin_model_enabled("bigmodel", "glm-5.3", true)
            .await
            .unwrap();
        assert_eq!(
            s.list().await.unwrap()[1].broken,
            None,
            "重新启用后恢复可用"
        );

        let custom = ModelConfig {
            provider_id: "deepseek".into(),
            api_mode: ApiMode::Anthropic,
            base_url: "https://api.deepseek.com/anthropic".into(),
            model: "deepseek-chat".into(),
            credential_ref: "model/preset-test".into(),
            id: "custom-1".into(),
            name: "自定义".into(),
            ..Default::default()
        };
        s.models.save_model(custom, None).await.unwrap();
        s.save(preset("q", Some("custom-1"), None, None))
            .await
            .unwrap();
        s.models.remove_model("custom-1").await.unwrap();
        assert!(
            s.list().await.unwrap()[2]
                .broken
                .as_deref()
                .unwrap()
                .contains("不存在"),
            "模型删除后预设标记 broken"
        );
    }
}
