//! Local stdio JSON-RPC. Only explicit human/API requests can synchronize source files.
pub mod acp;
pub mod approval;
pub mod chat;
pub mod console;
pub mod diffview;
pub mod line_editor;
pub mod manage;
pub mod run;
pub mod scope;
pub mod slash;

use harness_contracts::*;
use harness_runtime::{Runtime, model::ModelProvider};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::new("CLI", e.to_string())
}
fn field<'a>(p: &'a Value, key: &str) -> AppResult<&'a str> {
    p[key]
        .as_str()
        .ok_or_else(|| AppError::new("INVALID_PARAMS", key))
}
pub async fn provider(runtime: &Runtime, id: &str) -> AppResult<Arc<dyn ModelProvider>> {
    runtime.models.build_model(id).await
}

/// 按可选 effort 档位构建模型（None 跟随模型默认）。
pub async fn provider_with_effort(
    runtime: &Runtime,
    id: &str,
    effort: Option<&str>,
) -> AppResult<Arc<dyn ModelProvider>> {
    runtime
        .models
        .build_model_with(id, effort.map(str::to_owned))
        .await
}

pub async fn dispatch(runtime: Arc<Runtime>, method: &str, p: Value) -> AppResult<Value> {
    Ok(match method {
        "initialize" => {
            json!({"name":"Rust Harness","version":"0.1.0","protocol":"rust-harness.jsonrpc.v1"})
        }
        "workspace.list" => json!(runtime.workspaces.list()?),
        "workspace.open" => json!(
            runtime
                .workspaces
                .create(&PathBuf::from(field(&p, "path")?))?
        ),
        "workspace.diff" => json!(runtime.workspaces.diff(field(&p, "workspaceId")?)?),
        "workspace.apply" => {
            let changes: Vec<FileChange> =
                serde_json::from_value(p["reviewed"].clone()).map_err(err)?;
            json!(
                runtime
                    .workspaces
                    .apply_reviewed(field(&p, "workspaceId")?, &changes)?
            )
        }
        "session.list" => {
            json!(runtime.list_sessions(p["workspaceId"].as_str()).await?)
        }
        "session.active" => json!(runtime.active_ids()),
        "session.create" => {
            let w = field(&p, "workspaceId")?;
            runtime.workspaces.get(w)?;
            json!(
                runtime
                    .store
                    .create_session(w, p["title"].as_str().unwrap_or("新会话"))
                    .await?
            )
        }
        "session.events" => json!(
            runtime
                .store
                .events(
                    field(&p, "sessionId")?,
                    p["after"].as_str().unwrap_or("0").parse().map_err(err)?
                )
                .await?
        ),
        "session.send" => {
            // 分层：显式 modelId/effort > presetId 预设字段；均无模型时报错列出可用项。
            // 编程接口不读 cli.default_preset，预设仅随显式 presetId 生效。
            let preset = match p.get("presetId").and_then(Value::as_str) {
                Some(id) => Some(runtime.presets.resolve(id).await?),
                None => None,
            };
            let effort = p
                .get("effort")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| crate::manage::preset_effort(&preset));
            let model = match p.get("modelId").and_then(Value::as_str) {
                Some(id) => provider_with_effort(&runtime, id, effort.as_deref()).await?,
                None => {
                    let model_id = crate::manage::resolve_model_id(
                        &runtime,
                        crate::manage::preset_model(&preset),
                    )
                    .await?;
                    provider_with_effort(&runtime, &model_id, effort.as_deref()).await?
                }
            };
            runtime
                .run_session_with_attachments(
                    field(&p, "sessionId")?.into(),
                    field(&p, "text")?.into(),
                    model,
                    serde_json::from_value(p.get("attachments").cloned().unwrap_or(json!([])))
                        .map_err(err)?,
                )
                .await?;
            Value::Null
        }
        "artifact.import" => json!(
            runtime
                .artifact_import(field(&p, "sessionId")?, &PathBuf::from(field(&p, "path")?))
                .await?
        ),
        "artifact.read" => {
            let offset = p["offset"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| err("offset"))?;
            let length = p["length"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| err("length"))?;
            json!(
                runtime
                    .artifact_read(field(&p, "sessionId")?, field(&p, "hash")?, offset, length)
                    .await?
            )
        }
        "artifact.save" => {
            runtime
                .artifact_save(
                    field(&p, "sessionId")?,
                    field(&p, "hash")?,
                    &PathBuf::from(field(&p, "path")?),
                )
                .await?;
            Value::Null
        }
        "session.cancel" => {
            runtime.cancel(field(&p, "sessionId")?);
            Value::Null
        }
        "session.fork" => json!(runtime.fork(field(&p, "sessionId")?).await?),
        "session.export" => json!(runtime.store.export(field(&p, "sessionId")?).await?),
        "session.import" => {
            let w = field(&p, "workspaceId")?;
            runtime.workspaces.get(w)?;
            json!(runtime.store.import(field(&p, "content")?, w).await?)
        }
        "session.search" => json!(runtime.store.search(field(&p, "query")?).await?),
        "session.compact" => {
            let model = provider(&runtime, field(&p, "modelId")?).await?;
            harness_orchestration::compact_session(
                runtime,
                field(&p, "sessionId")?,
                model,
                Default::default(),
            )
            .await?;
            Value::Null
        }
        "approval.list" => json!(runtime.approvals.list()),
        "approval.resolve" => {
            runtime.approvals.resolve(
                field(&p, "id")?,
                p["allow"]
                    .as_bool()
                    .ok_or_else(|| AppError::new("INVALID_PARAMS", "allow"))?,
            )?;
            Value::Null
        }
        "provider.list" => json!(runtime.models.list_providers().await?),
        "provider.save" => json!(
            runtime
                .models
                .save_provider(
                    serde_json::from_value(p["config"].clone()).map_err(err)?,
                    p["secret"].as_str().map(str::to_owned)
                )
                .await?
        ),
        "provider.copy" => json!(runtime.models.copy_provider(field(&p, "id")?).await?),
        "provider.remove" => {
            runtime.models.remove_provider(field(&p, "id")?).await?;
            Value::Null
        }
        "model.list" => json!(runtime.models.list_models().await?),
        "model.save" => json!(
            runtime
                .models
                .save_model(
                    serde_json::from_value(p["config"].clone()).map_err(err)?,
                    p["secret"].as_str().map(str::to_owned)
                )
                .await?
        ),
        "model.remove" => {
            runtime.models.remove_model(field(&p, "modelId")?).await?;
            Value::Null
        }
        "model.test" => {
            runtime.models.test_model(field(&p, "modelId")?).await?;
            json!("连接成功")
        }
        "model.fetchAvailable" => json!(
            runtime
                .models
                .fetch_available_models(
                    field(&p, "providerId")?,
                    serde_json::from_value(p["apiMode"].clone()).map_err(err)?
                )
                .await?
        ),
        "model.setBuiltinEnabled" => {
            runtime
                .models
                .set_builtin_model_enabled(
                    field(&p, "providerId")?,
                    field(&p, "modelId")?,
                    p["enabled"]
                        .as_bool()
                        .ok_or_else(|| AppError::new("INVALID_PARAMS", "enabled"))?,
                )
                .await?;
            Value::Null
        }
        "preset.list" => json!(runtime.presets.list().await?),
        "preset.save" => json!(
            runtime
                .presets
                .save(serde_json::from_value(p["preset"].clone()).map_err(err)?)
                .await?
        ),
        "preset.remove" => {
            runtime.presets.remove(field(&p, "id")?).await?;
            Value::Null
        }
        "plugin.list" => json!(runtime.plugins.list()?),
        "plugin.install" => {
            if runtime.active_count() > 0 {
                return Err(AppError::new("BUSY", "请等待Agent回合结束"));
            }
            let grants = serde_json::from_value(p["permissions"].clone()).map_err(err)?;
            json!(
                runtime
                    .plugins
                    .install(&PathBuf::from(field(&p, "path")?), grants)?
            )
        }
        "plugin.enable" => {
            runtime.plugins.set_enabled(
                field(&p, "id")?,
                p["enabled"]
                    .as_bool()
                    .ok_or_else(|| AppError::new("INVALID_PARAMS", "enabled"))?,
            )?;
            Value::Null
        }
        "plugin.remove" => {
            runtime.plugins.remove(field(&p, "id")?)?;
            Value::Null
        }
        "mcp.list" => json!(runtime.mcp_connections()),
        "mcp.connect" => {
            runtime
                .connect_mcp(
                    field(&p, "workspaceId")?,
                    serde_json::from_value(p["config"].clone()).map_err(err)?,
                )
                .await?;
            Value::Null
        }
        "mcp.disconnect" => {
            runtime.disconnect_mcp(field(&p, "id")?).await?;
            Value::Null
        }
        "lsp.status" => {
            let all = runtime.lsp_status().await;
            match p.get("workspaceId").and_then(Value::as_str) {
                Some(workspace) => json!(
                    all.into_iter()
                        .filter(|e| e.workspace_id == workspace)
                        .collect::<Vec<_>>()
                ),
                None => json!(all),
            }
        }
        "lsp.restart" => {
            runtime
                .lsp_restart(field(&p, "workspaceId")?, field(&p, "serverId")?)
                .await?;
            Value::Null
        }
        _ => return Err(AppError::new("METHOD_NOT_FOUND", method)),
    })
}

pub async fn serve(runtime: Arc<Runtime>) -> AppResult<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(256);
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(v) = rx.recv().await {
            out.write_all(format!("{v}\n").as_bytes()).await?;
            out.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });
    let mut events = runtime.subscribe();
    let notify = tx.clone();
    let forward = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if notify
                .send(json!({"jsonrpc":"2.0","method":"session.event","params":event}))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut pending = tokio::task::JoinSet::new();
    while let Some(line) = lines.next_line().await.map_err(err)? {
        if line.len() > 1024 * 1024 {
            tx.send(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"request exceeds 1MiB"}})).await.map_err(err)?;
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                tx.send(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}})).await.map_err(err)?;
                continue;
            }
        };
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let params = request.get("params").cloned().unwrap_or(json!({}));
        let tx = tx.clone();
        let runtime = runtime.clone();
        pending.spawn(async move{let result=dispatch(runtime,&method,params).await;if let Some(id)=id{let response=match result{Ok(value)=>json!({"jsonrpc":"2.0","id":id,"result":value}),Err(e)=>json!({"jsonrpc":"2.0","id":id,"error":{"code":if e.code=="METHOD_NOT_FOUND"{-32601}else{-32000},"message":e.message,"data":{"code":e.code}}})};let _=tx.send(response).await;}});
        while pending.try_join_next().is_some() {}
    }
    runtime.cancel_all();
    while pending.join_next().await.is_some() {}
    forward.abort();
    drop(tx);
    writer.await.map_err(err)?.map_err(err)
}
