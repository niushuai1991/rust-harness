//! Local stdio JSON-RPC. Only explicit human/API requests can synchronize source files.
use harness_contracts::*;
use harness_runtime::{
    Runtime,
    model::{HttpModel, ModelProvider},
};
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
    let models: Vec<ModelConfig> =
        serde_json::from_value(runtime.store.get("models").await?.unwrap_or(json!([])))
            .map_err(err)?;
    let config = models
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::new("MODEL_NOT_FOUND", id))?;
    let secret = harness_platform::get_secret(&config.credential_ref)?.unwrap_or_default();
    Ok(Arc::new(HttpModel { config, secret }))
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
        "session.list" => json!(runtime.store.sessions(p["workspaceId"].as_str()).await?),
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
            let model = provider(&runtime, field(&p, "modelId")?).await?;
            runtime
                .run_session(
                    field(&p, "sessionId")?.into(),
                    field(&p, "text")?.into(),
                    model,
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
        "model.list" => runtime.store.get("models").await?.unwrap_or(json!([])),
        "model.save" => {
            let mut config: ModelConfig =
                serde_json::from_value(p["config"].clone()).map_err(err)?;
            let url = reqwest::Url::parse(&config.base_url).map_err(err)?;
            if !matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(AppError::new("MODEL_URL", "请输入不含凭据的HTTP(S)地址"));
            }
            if config.id.is_empty() {
                config.id = new_id();
            }
            config.credential_ref = format!("model/{}", config.id);
            if let Some(secret) = p["secret"].as_str().filter(|s| !s.is_empty()) {
                harness_platform::set_secret(&config.credential_ref, secret)?;
            }
            let mut all: Vec<ModelConfig> =
                serde_json::from_value(runtime.store.get("models").await?.unwrap_or(json!([])))
                    .map_err(err)?;
            all.retain(|m| m.id != config.id);
            all.push(config.clone());
            runtime.store.set("models", &json!(all)).await?;
            json!(config)
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
