//! SQLite is the authoritative session log; views and search indexes are disposable.
use harness_contracts::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, Row, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

/// 无项目会话（workspace_id 空串哨兵）沿用同一 schema；不新增迁移。
const SCHEMA_VERSION: i64 = 5;
/// 会话列表「项目」视图置顶工作区集合的 settings KV 键。
pub const PINNED_WORKSPACES_KEY: &str = "workspace.pinned.v1";
/// 已移除项目登记（重开同源路径时挂回旧会话与自动化）的 settings KV 键。
/// 项目移除（隐藏）集合的 settings KV 键；隐藏只影响任务列表入口显示，记录与会话原样保留。
pub const HIDDEN_WORKSPACES_KEY: &str = "workspace.hidden.v1";
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
                        endpoint_priority: None,
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
        if version <= 3 {
            let mut tx = pool.begin().await.map_err(db_error)?;
            sqlx::query("ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            sqlx::query("ALTER TABLE sessions ADD COLUMN archived_at TEXT")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            sqlx::query("PRAGMA user_version = 4")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
        }
        if version <= 4 {
            let mut tx = pool.begin().await.map_err(db_error)?;
            // 会话固化的执行位置；历史行保持 NULL，运行时按旧优先级回退。
            sqlx::query("ALTER TABLE sessions ADD COLUMN execution_mode TEXT")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            sqlx::query("PRAGMA user_version = 5")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
        }
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS events_session_kind_seq ON events(session_id,kind,seq)",
        )
        .execute(&pool)
        .await
        .map_err(db_error)?;
        let artifact_dir = path.parent().unwrap_or(Path::new(".")).join("artifacts");
        std::fs::create_dir_all(&artifact_dir).map_err(db_error)?;
        Ok(Self { pool, artifact_dir })
    }

    /// 创建会话。`workspace` 为空串表示「无项目」会话（哨兵值）：不挂任何项目，执行根由运行时按会话独立分配；标题校验与状态语义与普通会话一致。
    pub async fn create_session(&self, workspace: &str, title: &str) -> AppResult<SessionInfo> {
        self.create_session_with_mode(workspace, title, None).await
    }
    /// 创建会话并固化执行位置；`None` 表示跟随工作区默认（历史调用方与兼容路径）。
    pub async fn create_session_with_mode(
        &self,
        workspace: &str,
        title: &str,
        execution_mode: Option<ExecutionMode>,
    ) -> AppResult<SessionInfo> {
        if title.len() > 512 {
            return Err(AppError::new("INVALID_INPUT", "标题不得超过512字节"));
        }
        let title_state = if title == "新会话" {
            "default"
        } else {
            "manual"
        };
        let s = SessionInfo {
            archived_at: None,
            id: new_id(),
            workspace_id: workspace.into(),
            title: title.into(),
            created_at: now(),
            updated_at: now(),
            parent_id: None,
            execution_mode,
        };
        sqlx::query("INSERT INTO sessions(id,workspace_id,title,created_at,updated_at,title_state,execution_mode) VALUES(?,?,?,?,?,?,?)")
            .bind(&s.id)
            .bind(&s.workspace_id)
            .bind(&s.title)
            .bind(&s.created_at)
            .bind(&s.updated_at)
            .bind(title_state)
            .bind(execution_mode.map(mode_text))
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(s)
    }
    pub async fn session(&self, id: &str) -> AppResult<SessionInfo> {
        sqlx::query("SELECT id,workspace_id,title,created_at,updated_at,archived_at,execution_mode FROM sessions WHERE id=?")
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
    /// 归档会话：默认列表隐藏并记录归档时刻；重复归档保持原状态（幂等）。
    pub async fn archive_session(&self, id: &str) -> AppResult<SessionInfo> {
        let result =
            sqlx::query("UPDATE sessions SET archived=1,archived_at=? WHERE id=? AND archived=0")
                .bind(now())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return self.session(id).await;
        }
        self.session(id).await
    }
    /// 恢复归档会话：回到默认列表并清空归档时刻；不存在或未归档报 NOT_FOUND。
    pub async fn restore_session(&self, id: &str) -> AppResult<SessionInfo> {
        let result = sqlx::query(
            "UPDATE sessions SET archived=0,archived_at=NULL WHERE id=? AND archived=1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::new("NOT_FOUND", "会话不存在或未归档"));
        }
        self.session(id).await
    }
    /// 项目移除（隐藏）集合（settings KV，跨重启保留）；与置顶同模式，只影响任务列表入口显示。
    pub async fn hidden_workspaces(&self) -> AppResult<Vec<String>> {
        Ok(self
            .get(HIDDEN_WORKSPACES_KEY)
            .await?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }
    pub async fn set_workspace_hidden(&self, workspace: &str, hidden: bool) -> AppResult<()> {
        let mut ids = self.hidden_workspaces().await?;
        if hidden {
            if !ids.iter().any(|id| id == workspace) {
                ids.push(workspace.to_string());
            }
        } else {
            ids.retain(|id| id != workspace);
        }
        self.set(HIDDEN_WORKSPACES_KEY, &json!(ids)).await
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
    /// 全量会话列表（含归档）：保持既有查询语义不变，恢复/检索回退与子代理推导共用。
    pub async fn sessions(&self, workspace: Option<&str>) -> AppResult<Vec<SessionInfo>> {
        self.sessions_filtered(workspace, Some(false)).await
    }
    /// 按归档状态过滤的会话列表：`workspace` 过滤工作区；`archived` 为 None=仅活跃，Some(true)=仅归档，Some(false)=全部。
    pub async fn sessions_filtered(
        &self,
        workspace: Option<&str>,
        archived: Option<bool>,
    ) -> AppResult<Vec<SessionInfo>> {
        let rows = sqlx::query(
            "SELECT id,workspace_id,title,created_at,updated_at,archived_at,execution_mode FROM sessions \
             WHERE (? IS NULL OR workspace_id=?)
              AND ((? IS NULL AND archived=0) OR (? = 1 AND archived=1) OR (? = 0)) \
             ORDER BY updated_at DESC",
        )
        .bind(workspace)
        .bind(workspace)
        .bind(archived)
        .bind(archived)
        .bind(archived)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(rows.iter().map(session_row).collect())
    }
    /// 项目合并的会话迁移：把源工作区的全部会话（含归档）挂到目标工作区，
    /// 返回迁移数；源工作区无会话时为幂等空操作。事件与 FTS 索引按会话 id
    /// 关联，不随 workspace_id 变化。
    pub async fn move_sessions(&self, from: &str, to: &str) -> AppResult<u64> {
        if from.is_empty() || to.is_empty() {
            return Err(AppError::new("INVALID_INPUT", "工作区不能为空"));
        }
        let result = sqlx::query("UPDATE sessions SET workspace_id=? WHERE workspace_id=?")
            .bind(to)
            .bind(from)
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected())
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
        rows.iter().map(event_row).collect()
    }
    /// 已持久化但尚未结算的插话，按受理顺序返回。
    pub async fn pending_steering(&self, session: &str, run_id: &str) -> AppResult<Vec<Value>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT q.data FROM events q WHERE q.session_id=? AND q.kind='steer.queued' AND json_extract(q.data,'$.runId')=? AND NOT EXISTS (SELECT 1 FROM events s WHERE s.session_id=q.session_id AND s.kind IN ('steer.applied','steer.failed') AND json_extract(s.data,'$.runId')=json_extract(q.data,'$.runId') AND json_extract(s.data,'$.id')=json_extract(q.data,'$.id')) ORDER BY q.seq")
            .bind(session).bind(run_id).fetch_all(&self.pool).await.map_err(db_error)?;
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).map_err(db_error))
            .collect()
    }
    pub async fn turn_assistant_content(
        &self,
        session: &str,
        start: u64,
        end: u64,
    ) -> AppResult<Vec<Value>> {
        let start = i64::try_from(start).map_err(db_error)?;
        let end = i64::try_from(end).map_err(db_error)?;
        let start_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE session_id=? AND seq=? AND kind='turn.start')")
            .bind(session).bind(start).fetch_one(&self.pool).await.map_err(db_error)?;
        let boundary = sqlx::query("SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.start' AND seq>? ORDER BY seq LIMIT 1) UNION ALL SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.end' AND seq>? ORDER BY seq LIMIT 1) ORDER BY seq LIMIT 1")
            .bind(session).bind(start).bind(session).bind(start).fetch_optional(&self.pool).await.map_err(db_error)?
            .as_ref().map(event_row).transpose()?;
        if !start_exists
            || !boundary.as_ref().is_some_and(|e| {
                e.kind == "turn.end" && e.seq == end.to_string() && e.data["status"] == "completed"
            })
        {
            return Err(AppError::new("INVALID_TURN", "无法确认完整回合"));
        }
        let bytes: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(LENGTH(CAST(data AS BLOB))),0) FROM events WHERE session_id=? AND kind='assistant.message' AND seq>? AND seq<?")
            .bind(session).bind(start).bind(end).fetch_one(&self.pool).await.map_err(db_error)?;
        if bytes > 8 * 1024 * 1024 {
            return Err(AppError::new(
                "TURN_COPY_TOO_LARGE",
                "回合内容过大，无法复制（上限 8 MiB）",
            ));
        }
        let rows: Vec<String> = sqlx::query_scalar("SELECT data FROM events WHERE session_id=? AND kind='assistant.message' AND seq>? AND seq<? ORDER BY seq")
            .bind(session).bind(start).bind(end).fetch_all(&self.pool).await.map_err(db_error)?;
        rows.iter()
            .map(|raw| {
                let data: Value = serde_json::from_str(raw).map_err(db_error)?;
                Ok(data.get("content").cloned().unwrap_or(Value::Null))
            })
            .collect()
    }
    /// User message previews in ascending sequence order, bounded by an exclusive cursor.
    pub async fn turn_navigator_page(
        &self,
        session: &str,
        before: Option<u64>,
        limit: Option<u16>,
    ) -> AppResult<TurnNavigatorPage> {
        let before = before
            .map(|v| i64::try_from(v).map_err(|_| AppError::new("INVALID_CURSOR", "游标溢出")))
            .transpose()?;
        let limit = limit.unwrap_or(100).clamp(1, 200);
        let mut rows = sqlx::query("SELECT id, seq, data FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='user.message' AND (? IS NULL OR seq<?) ORDER BY seq DESC LIMIT ?")
            .bind(session).bind(before).bind(before).bind(i64::from(limit)+1).fetch_all(&self.pool).await.map_err(db_error)?;
        let has_older = rows.len() > usize::from(limit);
        if has_older {
            rows.pop();
        }
        rows.reverse();
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let data: Value =
                serde_json::from_str(row.try_get::<&str, _>("data").map_err(db_error)?)
                    .map_err(db_error)?;
            let normalized = data
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let text = if normalized.is_empty() {
                "用户输入"
            } else {
                &normalized
            };
            let mut graphemes = text.graphemes(true);
            let preview = graphemes.by_ref().take(160).collect::<String>();
            entries.push(TurnNavigatorEntry {
                event_id: row.try_get("id").map_err(db_error)?,
                seq: row.try_get::<i64, _>("seq").map_err(db_error)?.to_string(),
                preview,
                preview_truncated: graphemes.next().is_some(),
            });
        }
        let older_cursor = if has_older {
            entries.first().map(|e| e.seq.clone())
        } else {
            None
        };
        Ok(TurnNavigatorPage {
            entries,
            older_cursor,
            has_older,
        })
    }
    /// Returns a bounded durable-event window for the desktop timeline.
    pub async fn timeline_page(
        &self,
        session: &str,
        before: Option<u64>,
        limit: Option<u16>,
    ) -> AppResult<TimelinePage> {
        let through: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM events WHERE session_id=?")
                .bind(session)
                .fetch_one(&self.pool)
                .await
                .map_err(db_error)?;
        let through = through.unwrap_or(0);
        if through == 0 {
            return Ok(TimelinePage {
                session_id: session.into(),
                events: vec![],
                older_cursor: None,
                through_seq: "0".into(),
                has_older: false,
                turns: vec![],
            });
        }
        let before = before
            .map(|cursor| {
                i64::try_from(cursor).map_err(|_| AppError::new("INVALID_CURSOR", "游标溢出"))
            })
            .transpose()?;
        let limit = limit
            .unwrap_or(DEFAULT_TIMELINE_PAGE_SIZE)
            .clamp(1, MAX_TIMELINE_PAGE_SIZE);
        // 侧聊继承边界：边界及之前的复制事件、边界事件与合成说明消息只供模型参考，
        // 不进界面时间线（全量 events/export 不受影响）。
        let side_before: Option<i64> =
            sqlx::query_scalar("SELECT CAST(json_extract(data,'$.beforeSeq') AS INTEGER) FROM events WHERE session_id=? AND kind='side.boundary' ORDER BY seq LIMIT 1")
                .bind(session)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_error)?
                .flatten();
        let mut rows = sqlx::query(
            "SELECT * FROM events WHERE session_id=? AND seq<=? AND (? IS NULL OR seq<?) \
             AND kind!='side.boundary' \
             AND (kind!='user.message' OR COALESCE(json_extract(data,'$.source'),'')!='side_inherited') \
             AND (? IS NULL OR seq>?) \
             ORDER BY seq DESC LIMIT ?",
        )
        .bind(session)
        .bind(through)
        .bind(before)
        .bind(before)
        .bind(side_before)
        .bind(side_before)
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        let has_older = rows.len() > usize::from(limit);
        if has_older {
            rows.pop();
        }
        let mut events = rows.iter().map(event_row).collect::<AppResult<Vec<_>>>()?;
        events.reverse();
        for event in &mut events {
            if event.kind == "steer.queued"
                && let (Some(id), Some(run_id)) =
                    (event.data["id"].as_str(), event.data["runId"].as_str())
            {
                let status = sqlx::query_scalar::<_, String>("SELECT kind FROM events WHERE session_id=? AND seq<=? AND kind IN ('steer.applied','steer.failed') AND json_extract(data,'$.id')=? AND json_extract(data,'$.runId')=? ORDER BY seq DESC LIMIT 1")
                    .bind(session).bind(through).bind(id).bind(run_id).fetch_optional(&self.pool).await.map_err(db_error)?;
                if let Some(status) = status {
                    event.data["steerStatus"] = json!(status);
                }
            }
        }
        let older_cursor = has_older.then(|| {
            events
                .first()
                .expect("a page with earlier events cannot be empty")
                .seq
                .clone()
        });
        let mut starts = Vec::new();
        if let Some(first) = events.first() {
            let first_seq = first.seq.parse::<i64>().map_err(db_error)?;
            let prior = sqlx::query("SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.start' AND seq<? AND (? IS NULL OR seq>?) ORDER BY seq DESC LIMIT 1) UNION ALL SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.end' AND seq<? AND (? IS NULL OR seq>?) ORDER BY seq DESC LIMIT 1) ORDER BY seq DESC LIMIT 1")
                .bind(session).bind(first_seq).bind(side_before).bind(side_before).bind(session).bind(first_seq).bind(side_before).bind(side_before).fetch_optional(&self.pool).await.map_err(db_error)?;
            if let Some(row) = prior {
                let event = event_row(&row)?;
                if event.kind == "turn.start" {
                    starts.push(event);
                }
            }
        }
        starts.extend(events.iter().filter(|e| e.kind == "turn.start").cloned());
        let mut turns = Vec::new();
        for start in starts {
            let seq = start.seq.parse::<i64>().map_err(db_error)?;
            let boundary = sqlx::query("SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.start' AND seq>? AND seq<=? ORDER BY seq LIMIT 1) UNION ALL SELECT * FROM (SELECT * FROM events INDEXED BY events_session_kind_seq WHERE session_id=? AND kind='turn.end' AND seq>? AND seq<=? ORDER BY seq LIMIT 1) ORDER BY seq LIMIT 1")
                .bind(session).bind(seq).bind(through).bind(session).bind(seq).bind(through).fetch_optional(&self.pool).await.map_err(db_error)?
                .as_ref().map(event_row).transpose()?;
            let end = boundary.filter(|event| event.kind == "turn.end");
            let final_message_id = if let Some(ref end) = end {
                let end_seq = end.seq.parse::<i64>().map_err(db_error)?;
                sqlx::query_scalar::<_, String>("SELECT id FROM events WHERE session_id=? AND kind='assistant.message' AND seq>? AND seq<? ORDER BY seq DESC LIMIT 1")
                    .bind(session).bind(seq).bind(end_seq).fetch_optional(&self.pool).await.map_err(db_error)?
            } else {
                None
            };
            turns.push(TimelineTurn {
                start_seq: start.seq,
                run_id: start.data["runId"].as_str().map(str::to_owned),
                end_seq: end.as_ref().map(|e| e.seq.clone()),
                status: end
                    .as_ref()
                    .and_then(|e| e.data.get("status"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                final_message_id,
            });
        }
        Ok(TimelinePage {
            session_id: session.into(),
            events,
            older_cursor,
            through_seq: through.to_string(),
            has_older,
            turns,
        })
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
            let settled_steers: std::collections::HashSet<(&str, &str)> = events
                .iter()
                .filter(|event| matches!(event.kind.as_str(), "steer.applied" | "steer.failed"))
                .filter_map(|event| {
                    Some((event.data["runId"].as_str()?, event.data["id"].as_str()?))
                })
                .collect();
            for event in events.iter().filter(|event| event.kind == "steer.queued") {
                if let (Some(run), Some(id)) =
                    (event.data["runId"].as_str(), event.data["id"].as_str())
                    && !settled_steers.contains(&(run, id))
                {
                    records.push((
                        "steer.failed".into(),
                        json!({"runId":run,"id":id,"reason":"应用中断，插话未处理"}),
                    ));
                }
            }
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
    /// 统计现存 `request.snapshot` 事件的行数与字节占用（dry-run 预览用），不执行删除。
    pub async fn request_snapshot_usage(&self) -> AppResult<PurgeSummary> {
        let (rows, bytes): (i64, Option<i64>) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(data)),0) FROM events WHERE kind='request.snapshot'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(PurgeSummary {
            rows: u64::try_from(rows).unwrap_or_default(),
            bytes: u64::try_from(bytes.unwrap_or_default()).unwrap_or_default(),
        })
    }
    /// 删除全部会话的历史 `request.snapshot` 事件：该 kind 曾在每轮模型请求前把
    /// 全量对话历史与 tools 整体落库（事件本身无消费方），是 events 表膨胀的主因。
    /// 只动 events 表；FTS 索引仅含消息事件，无需同步。
    pub async fn purge_request_snapshots(&self) -> AppResult<PurgeSummary> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        let (rows, bytes): (i64, Option<i64>) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(data)),0) FROM events WHERE kind='request.snapshot'",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        sqlx::query("DELETE FROM events WHERE kind='request.snapshot'")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(PurgeSummary {
            rows: u64::try_from(rows).unwrap_or_default(),
            bytes: u64::try_from(bytes.unwrap_or_default()).unwrap_or_default(),
        })
    }
    /// 执行 `VACUUM` 把已删除页归还操作系统；不能在事务中运行，且要求无并发读锁，
    /// 因此用独立连接执行（池内连接可能残留未 drain 的语句游标导致 SQLITE_BUSY）。
    pub async fn vacuum(&self) -> AppResult<()> {
        let options = self.pool.connect_options().clone();
        let mut conn = SqliteConnection::connect_with(&options)
            .await
            .map_err(db_error)?;
        sqlx::raw_sql("VACUUM")
            .execute(&mut conn)
            .await
            .map_err(db_error)?;
        conn.close().await.map_err(db_error)?;
        Ok(())
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
        // Import requires contiguous sequence numbers; translate compaction boundaries too.
        let mut exported_seq = 0usize;
        let mut sequence_map = BTreeMap::new();
        for mut e in self.events(id, 0).await? {
            let original_seq = e.seq.parse::<u64>().map_err(db_error)?;
            if e.kind == "request.snapshot" {
                sequence_map.insert(original_seq, exported_seq);
                continue;
            }
            if e.kind == "compaction.summary"
                && let Some(through_seq) = e.data["throughSeq"].as_str()
                && let Ok(through_seq) = through_seq.parse::<u64>()
            {
                let mapped = sequence_map
                    .range(..=through_seq)
                    .next_back()
                    .map_or(0, |(_, seq)| *seq);
                e.data["throughSeq"] = json!(mapped.to_string());
            }
            exported_seq += 1;
            sequence_map.insert(original_seq, exported_seq);
            e.seq = exported_seq.to_string();
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
            archived_at: None,
            id: new_id(),
            workspace_id: workspace_id.into(),
            title: header["session"]["title"]
                .as_str()
                .unwrap_or("导入会话")
                .into(),
            created_at: now(),
            updated_at: now(),
            parent_id: None,
            // 导入会话不带执行位置，跟随导入后工作区的默认值。
            execution_mode: None,
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
        archived_at: r.get("archived_at"),
        parent_id: None,
        execution_mode: r
            .get::<Option<String>, _>("execution_mode")
            .as_deref()
            .and_then(mode_from_text),
    }
}

/// 执行位置在库内的稳定文本表示（与 serde camelCase 一致）。
fn mode_text(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Direct => "direct",
        ExecutionMode::Replica => "replica",
        ExecutionMode::Worktree => "worktree",
    }
}

/// 解析库内执行位置；未知值（未来版本写入）按 None 处理，回退到工作区默认。
fn mode_from_text(text: &str) -> Option<ExecutionMode> {
    match text {
        "direct" => Some(ExecutionMode::Direct),
        "replica" => Some(ExecutionMode::Replica),
        "worktree" => Some(ExecutionMode::Worktree),
        _ => None,
    }
}

fn event_row(r: &sqlx::sqlite::SqliteRow) -> AppResult<SessionEvent> {
    Ok(SessionEvent {
        id: r.get("id"),
        session_id: r.get("session_id"),
        seq: r.get::<i64, _>("seq").to_string(),
        kind: r.get("kind"),
        data: serde_json::from_str(r.get("data")).map_err(db_error)?,
        created_at: r.get("created_at"),
    })
}
