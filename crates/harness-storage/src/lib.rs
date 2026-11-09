//! SQLite is the authoritative session log; views and search indexes are disposable.
use harness_contracts::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 3;
/// 会话列表「项目」视图置顶工作区集合的 settings KV 键。
pub const PINNED_WORKSPACES_KEY: &str = "workspace.pinned.v1";
fn db_error(e: impl std::fmt::Display) -> AppError {
    AppError::new("STORAGE", e.to_string())
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    artifact_dir: PathBuf,
}
impl Store {
    pub async fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(db_error)?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(10)),
            )
            .await
            .map_err(db_error)?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .map_err(db_error)?;
        if version > SCHEMA_VERSION {
            pool.close().await;
            return Err(AppError::new(
                "FUTURE_SCHEMA",
                format!("数据库版本 {version} 高于支持版本 {SCHEMA_VERSION}"),
            ));
        }
        if version == 0 {
            let mut tx = pool.begin().await.map_err(db_error)?;
            for sql in [
                "CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)",
                "CREATE TABLE IF NOT EXISTS events(session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,seq INTEGER NOT NULL,id TEXT NOT NULL UNIQUE,kind TEXT NOT NULL,data TEXT NOT NULL,created_at TEXT NOT NULL,PRIMARY KEY(session_id,seq))",
                "CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
                "CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(session_id UNINDEXED,content,tokenize='unicode61')",
                "PRAGMA user_version = 1",
            ] {
                sqlx::query(sql).execute(&mut *tx).await.map_err(db_error)?;
            }
            tx.commit().await.map_err(db_error)?;
        }
        if version <= 1 {
            // VACUUM INTO includes committed WAL pages; a plain file copy does not.
            if version == 1 {
                let mut backup = path.with_extension("db.v1.bak");
                if backup.exists() {
                    backup = path.with_extension(format!("db.v1.{}.bak", new_id()));
                }
                sqlx::query("VACUUM INTO ?")
                    .bind(backup.to_string_lossy().as_ref())
                    .execute(&pool)
                    .await
                    .map_err(db_error)?;
            }
            let mut tx = pool.begin().await.map_err(db_error)?;
            let raw: Option<String> =
                sqlx::query_scalar("SELECT value FROM settings WHERE key='models'")
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(db_error)?;
            if let Some(raw) = raw {
                let mut models: Vec<ModelConfig> = serde_json::from_str(&raw).map_err(db_error)?;
                let existing: Option<String> =
                    sqlx::query_scalar("SELECT value FROM settings WHERE key='providers'")
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(db_error)?;
                let mut providers: Vec<ProviderInfo> = existing
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(db_error)?
                    .unwrap_or_default();
                for model in &mut models {
                    if !model.provider_id.is_empty() {
                        continue;
                    }
                    let id = format!("legacy-{}", model.id);
                    providers.push(ProviderInfo {
                        compatibility: ProviderCompatibility::Standard,
                        auth: ProviderAuth::ApiKey,
                        id: id.clone(),
                        name: model.name.clone(),
                        builtin: false,
                        endpoints: vec![ProtocolEndpoint {
                            api_mode: ApiMode::ChatCompletions,
                            base_url: model.base_url.clone(),
                        }],
                        default_model: model.model.clone(),
                        credential_ref: model.credential_ref.clone(),
                        proxy: model.proxy.clone(),
                        enabled: true,
                        builtin_models: Vec::new(),
                        disabled_models: Vec::new(),
                    });
                    model.provider_id = id;
                }
                for (key, value) in [("models", json!(models)), ("providers", json!(providers))] {
                    sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(key).bind(value.to_string()).execute(&mut *tx).await.map_err(db_error)?;
                }
            }
            sqlx::query("PRAGMA user_version = 2")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
        }
        if version <= 2 {
            let mut tx = pool.begin().await.map_err(db_error)?;
            for sql in [
                "CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,title TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)",
                "CREATE TABLE IF NOT EXISTS events(session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,seq INTEGER NOT NULL,id TEXT NOT NULL UNIQUE,kind TEXT NOT NULL,data TEXT NOT NULL,created_at TEXT NOT NULL,PRIMARY KEY(session_id,seq))",
                "CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
                "CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(session_id UNINDEXED,content,tokenize='unicode61')",
            ] {
                sqlx::query(sql).execute(&mut *tx).await.map_err(db_error)?;
            }
            sqlx::query(
                "ALTER TABLE sessions ADD COLUMN title_state TEXT NOT NULL DEFAULT 'manual'",
            )
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
            sqlx::query(
                "UPDATE sessions SET title_state='default' \
                 WHERE title='新会话' AND NOT EXISTS (\
                    SELECT 1 FROM events WHERE events.session_id=sessions.id AND kind='user.message'\
                 )",
            )
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
            sqlx::query("PRAGMA user_version = 3")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
        }
        let artifact_dir = path.parent().unwrap_or(Path::new(".")).join("artifacts");
        std::fs::create_dir_all(&artifact_dir).map_err(db_error)?;
        Ok(Self { pool, artifact_dir })
    }

    pub async fn create_session(&self, workspace: &str, title: &str) -> AppResult<SessionInfo> {
        if workspace.is_empty() || title.len() > 512 {
            return Err(AppError::new(
                "INVALID_INPUT",
                "工作区不能为空，标题不得超过512字节",
            ));
        }
        let title_state = if title == "新会话" {
            "default"
        } else {
            "manual"
        };
        let s = SessionInfo {
            id: new_id(),
            workspace_id: workspace.into(),
            title: title.into(),
            created_at: now(),
            updated_at: now(),
            parent_id: None,
        };
        sqlx::query("INSERT INTO sessions(id,workspace_id,title,created_at,updated_at,title_state) VALUES(?,?,?,?,?,?)")
            .bind(&s.id)
            .bind(&s.workspace_id)
            .bind(&s.title)
            .bind(&s.created_at)
            .bind(&s.updated_at)
            .bind(title_state)
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(s)
    }
    pub async fn session(&self, id: &str) -> AppResult<SessionInfo> {
        sqlx::query("SELECT id,workspace_id,title,created_at,updated_at FROM sessions WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_error)?
            .map(|r| session_row(&r))
            .ok_or_else(|| AppError::new("NOT_FOUND", "会话不存在"))
    }
    pub async fn session_title_auto_pending(&self, id: &str) -> AppResult<bool> {
        let state: Option<String> =
            sqlx::query_scalar("SELECT title_state FROM sessions WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_error)?;
        state
            .map(|state| state == "default")
            .ok_or_else(|| AppError::new("NOT_FOUND", "会话不存在"))
    }
    pub async fn rename_session(&self, id: &str, title: &str) -> AppResult<SessionInfo> {
        let title = title.trim();
        if title.is_empty() || title.len() > 512 {
            return Err(AppError::new(
                "INVALID_INPUT",
                "会话标题不能为空，且不得超过512字节",
            ));
        }
        let result = sqlx::query("UPDATE sessions SET title=?,title_state='manual' WHERE id=?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::new("NOT_FOUND", "会话不存在"));
        }
        self.session(id).await
    }
    pub async fn set_generated_title(&self, id: &str, title: &str) -> AppResult<SessionInfo> {
        let title = title.trim();
        if title.is_empty() || title.len() > 512 {
            return Err(AppError::new(
                "INVALID_INPUT",
                "会话标题不能为空，且不得超过512字节",
            ));
        }
        let result = sqlx::query(
            "UPDATE sessions SET title=?,title_state='generated' WHERE id=? AND title_state='default'",
        )
        .bind(title)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return self.session(id).await;
        }
        self.session(id).await
    }
    pub async fn sessions(&self, workspace: Option<&str>) -> AppResult<Vec<SessionInfo>> {
        let rows = sqlx::query(
            "SELECT id,workspace_id,title,created_at,updated_at FROM sessions \
             WHERE (? IS NULL OR workspace_id=?) ORDER BY updated_at DESC",
        )
        .bind(workspace)
        .bind(workspace)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(rows.iter().map(session_row).collect())
    }
    pub async fn append(&self, session: &str, kind: &str, data: Value) -> AppResult<SessionEvent> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        let event = append_tx(&mut tx, session, kind, data).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(event)
    }
    /// Atomically records a settled assistant and every tool call it admitted.
    pub async fn append_batch(
        &self,
        session: &str,
        records: Vec<(String, Value)>,
    ) -> AppResult<Vec<SessionEvent>> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        let mut events = Vec::new();
        for (kind, data) in records {
            events.push(append_tx(&mut tx, session, &kind, data).await?);
        }
        tx.commit().await.map_err(db_error)?;
        Ok(events)
    }
    pub async fn events(&self, session: &str, after: u64) -> AppResult<Vec<SessionEvent>> {
        let after =
            i64::try_from(after).map_err(|_| AppError::new("INVALID_CURSOR", "游标溢出"))?;
        let rows = sqlx::query("SELECT * FROM events WHERE session_id=? AND seq>? ORDER BY seq")
            .bind(session)
            .bind(after)
            .fetch_all(&self.pool)
            .await
            .map_err(db_error)?;
        rows.iter()
            .map(|r| {
                Ok(SessionEvent {
                    id: r.get("id"),
                    session_id: r.get("session_id"),
                    seq: r.get::<i64, _>("seq").to_string(),
                    kind: r.get("kind"),
                    data: serde_json::from_str(r.get("data")).map_err(db_error)?,
                    created_at: r.get("created_at"),
                })
            })
            .collect()
    }
    /// Called before accepting work. Recovery records uncertainty, never invokes a tool.
    pub async fn recover(&self) -> AppResult<usize> {
        let mut recovered = 0;
        for s in self.sessions(None).await? {
            let events = self.events(&s.id, 0).await?;
            let last_start = events.iter().rposition(|e| e.kind == "turn.start");
            let last_end = events.iter().rposition(|e| e.kind == "turn.end");
            let mut calls = std::collections::BTreeMap::new();
            let results = events
                .iter()
                .filter(|e| e.kind == "tool.result")
                .filter_map(|e| e.data["callId"].as_str())
                .collect::<std::collections::BTreeSet<_>>();
            for event in &events {
                if event.kind == "assistant.message" {
                    for call in event.data["toolCalls"].as_array().into_iter().flatten() {
                        if let Some(id) = call["id"].as_str() {
                            calls.insert(id.to_owned(), call.clone());
                        }
                    }
                }
                if event.kind == "tool.call"
                    && let Some(id) = event.data["id"].as_str()
                {
                    calls.insert(id.to_owned(), event.data.clone());
                }
            }
            let mut records = Vec::new();
            for (id, call) in calls {
                if !results.contains(id.as_str()) {
                    if !events
                        .iter()
                        .any(|e| e.kind == "tool.call" && e.data["id"] == id)
                    {
                        records.push(("tool.call".into(), call.clone()));
                    }
                    records.push(("tool.result".into(),json!({"callId":id,"name":call["name"],"content":"进程中断；外部操作是否完成未知，未自动重试。","isError":true,"status":"unknown"})));
                }
            }
            if let Some(start) = last_start.filter(|start| last_end.is_none_or(|end| end < *start))
            {
                records.push((
                    "turn.end".into(),
                    json!({"runId":events[start].data["runId"],"status":"interrupted"}),
                ));
            }
            if !records.is_empty() {
                self.append_batch(&s.id, records).await?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
    /// Latest todo/plan/goal entry per session for the desktop overview panel; derived view.
    pub async fn latest_states(&self) -> AppResult<Vec<PlanStateOverview>> {
        let rows = sqlx::query(
            "SELECT e.session_id,e.kind,e.data,e.created_at,s.title,s.workspace_id FROM events e \
             JOIN (SELECT session_id,kind,MAX(seq) AS seq FROM events \
                   WHERE kind IN ('todo.state','plan.state','goal.state') GROUP BY session_id,kind) m \
             ON e.session_id=m.session_id AND e.kind=m.kind AND e.seq=m.seq \
             JOIN sessions s ON s.id=e.session_id \
             ORDER BY s.updated_at DESC,e.kind",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        rows.iter()
            .map(|r| {
                Ok(PlanStateOverview {
                    session_id: r.get("session_id"),
                    session_title: r.get("title"),
                    workspace_id: r.get("workspace_id"),
                    kind: r.get("kind"),
                    data: serde_json::from_str(r.get("data")).map_err(db_error)?,
                    updated_at: r.get("created_at"),
                })
            })
            .collect()
    }
    pub async fn search(&self, query: &str) -> AppResult<Vec<SessionInfo>> {
        if query.trim().is_empty() {
            return self.sessions(None).await;
        }
        let literal = format!("\"{}\"", query.replace('"', "\"\""));
        let rows = sqlx::query("SELECT DISTINCT s.* FROM sessions s JOIN message_search m ON s.id=m.session_id WHERE message_search MATCH ? ORDER BY s.updated_at DESC LIMIT 100")
            .bind(literal).fetch_all(&self.pool).await.map_err(db_error)?;
        Ok(rows.iter().map(session_row).collect())
    }
    pub async fn get(&self, key: &str) -> AppResult<Option<Value>> {
        let text: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_error)?;
        text.map(|s| serde_json::from_str(&s).map_err(db_error))
            .transpose()
    }
    pub async fn set(&self, key: &str, value: &Value) -> AppResult<()> {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(key).bind(value.to_string()).execute(&self.pool).await.map_err(db_error)?;
        Ok(())
    }
    /// 会话列表「项目」视图的置顶工作区集合（settings KV，跨重启保留）。
    pub async fn pinned_workspaces(&self) -> AppResult<Vec<String>> {
        Ok(self
            .get(PINNED_WORKSPACES_KEY)
            .await?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }
    pub async fn set_workspace_pinned(&self, workspace: &str, pinned: bool) -> AppResult<()> {
        let mut ids = self.pinned_workspaces().await?;
        if pinned {
            if !ids.iter().any(|id| id == workspace) {
                ids.push(workspace.to_string());
            }
        } else {
            ids.retain(|id| id != workspace);
        }
        self.set(PINNED_WORKSPACES_KEY, &json!(ids)).await
    }
    pub async fn export(&self, id: &str) -> AppResult<String> {
        let mut lines = vec![
            json!({"format":"rust-harness.session","version":1,"session":self.session(id).await?})
                .to_string(),
        ];
        for e in self.events(id, 0).await? {
            lines.push(serde_json::to_string(&e).map_err(db_error)?);
        }
        Ok(lines.join("\n") + "\n")
    }
    pub async fn import(&self, input: &str, workspace_id: &str) -> AppResult<SessionInfo> {
        if input.len() > 16 * 1024 * 1024 {
            return Err(AppError::new("IMPORT_LIMIT", "会话导入超过16MiB"));
        }
        let mut lines = input.lines();
        let header: Value = serde_json::from_str(lines.next().unwrap_or("")).map_err(db_error)?;
        if header["format"] != "rust-harness.session" || header["version"] != 1 {
            return Err(AppError::new("FORMAT", "不支持的会话格式"));
        }
        let parsed: Vec<SessionEvent> = lines
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .map_err(db_error)?;
        for (i, e) in parsed.iter().enumerate() {
            if e.seq != (i + 1).to_string() {
                return Err(AppError::new("FORMAT", "事件游标不连续"));
            }
        }
        let session = SessionInfo {
            id: new_id(),
            workspace_id: workspace_id.into(),
            title: header["session"]["title"]
                .as_str()
                .unwrap_or("导入会话")
                .into(),
            created_at: now(),
            updated_at: now(),
            parent_id: None,
        };
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("INSERT INTO sessions(id,workspace_id,title,created_at,updated_at,title_state) VALUES(?,?,?,?,?,'manual')")
            .bind(&session.id)
            .bind(&session.workspace_id)
            .bind(&session.title)
            .bind(&session.created_at)
            .bind(&session.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        for event in parsed {
            let mut data = event.data;
            if event.kind == "subagent.created" {
                if !data.is_object() {
                    return Err(AppError::new("FORMAT", "无效的子代理历史记录"));
                }
                data["historyOnly"] = json!(true);
            }
            append_tx(&mut tx, &session.id, &event.kind, data).await?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(session)
    }
    pub async fn put_blob(&self, bytes: &[u8]) -> AppResult<String> {
        let hash = hex::encode(Sha256::digest(bytes));
        let dest = self.artifact_dir.join(&hash);
        if !dest.exists() {
            let tmp = self.artifact_dir.join(new_id());
            tokio::fs::write(&tmp, bytes).await.map_err(db_error)?;
            if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
                let _ = tokio::fs::remove_file(&tmp).await;
                if !dest.exists() {
                    return Err(db_error(e));
                }
            }
        }
        Ok(hash)
    }
    pub async fn blob(&self, hash: &str) -> AppResult<Vec<u8>> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("INVALID_BLOB", "无效附件标识"));
        }
        let bytes = tokio::fs::read(self.artifact_dir.join(hash))
            .await
            .map_err(db_error)?;
        if hex::encode(Sha256::digest(&bytes)) != hash {
            return Err(AppError::new("CORRUPT_BLOB", "附件内容校验失败"));
        }
        Ok(bytes)
    }
}
async fn append_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session: &str,
    kind: &str,
    data: Value,
) -> AppResult<SessionEvent> {
    let id = new_id();
    let created = now();
    // Allocation and insertion are a single write statement, so concurrent appenders cannot reuse a cursor.
    let seq: i64 = sqlx::query_scalar("INSERT INTO events(session_id,seq,id,kind,data,created_at) SELECT ?,COALESCE(MAX(seq),0)+1,?,?,?,? FROM events WHERE session_id=? RETURNING seq")
            .bind(session).bind(&id).bind(kind).bind(data.to_string()).bind(&created).bind(session)
            .fetch_one(&mut **tx).await.map_err(db_error)?;
    sqlx::query("UPDATE sessions SET updated_at=? WHERE id=?")
        .bind(&created)
        .bind(session)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    if matches!(kind, "user.message" | "assistant.message")
        && let Some(content) = data.get("content").and_then(Value::as_str)
    {
        sqlx::query("INSERT INTO message_search(session_id,content) VALUES(?,?)")
            .bind(session)
            .bind(content)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
    }
    Ok(SessionEvent {
        id,
        session_id: session.into(),
        seq: seq.to_string(),
        kind: kind.into(),
        data,
        created_at: created,
    })
}
fn session_row(r: &sqlx::sqlite::SqliteRow) -> SessionInfo {
    SessionInfo {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        title: r.get("title"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        parent_id: None,
    }
}
