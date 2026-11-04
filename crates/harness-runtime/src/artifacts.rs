//! Session-scoped attachment access; host paths never enter model history.
use crate::Runtime;
use base64::{Engine, engine::general_purpose::STANDARD};
use harness_contracts::{AppError, AppResult, ArtifactChunk, ArtifactMetadata};
use serde_json::json;
use std::path::Path;
use tokio::io::AsyncReadExt;

fn error(message: impl ToString) -> AppError {
    AppError::new("ARTIFACT", message.to_string())
}
pub(crate) fn is_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| {
        !s.chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    })
}
impl Runtime {
    pub async fn artifact_import(&self, session: &str, path: &Path) -> AppResult<ArtifactMetadata> {
        self.store.session(session).await?;
        let file = tokio::fs::File::open(path).await.map_err(error)?;
        if !file.metadata().await.map_err(error)?.is_file() {
            return Err(error("只能导入普通文件"));
        }
        let mut bytes = Vec::new();
        file.take(20 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(error)?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(error("附件不能超过20MiB"));
        }
        let metadata = ArtifactMetadata {
            hash: self.store.put_blob(&bytes).await?,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            media_type: if is_text(&bytes) {
                "text/plain; charset=utf-8"
            } else {
                "application/octet-stream"
            }
            .into(),
            size: bytes.len() as u32,
        };
        self.append(session, "attachment.imported", json!({"artifact":metadata}))
            .await?;
        Ok(metadata)
    }
    pub(crate) async fn artifact_metadata(
        &self,
        session: &str,
        hash: &str,
    ) -> AppResult<ArtifactMetadata> {
        self.store.session(session).await?;
        for event in self.store.events(session, 0).await? {
            let values: Vec<_> = match event.kind.as_str() {
                "attachment.imported" | "tool.result" => vec![&event.data["artifact"]],
                "user.message" => event.data["attachments"]
                    .as_array()
                    .map(|a| a.iter().collect())
                    .unwrap_or_default(),
                _ => vec![],
            };
            for value in values {
                if value["hash"].as_str() == Some(hash) {
                    return serde_json::from_value(value.clone()).map_err(error);
                }
            }
        }
        Err(AppError::new("ARTIFACT_SCOPE", "该附件不属于当前会话"))
    }
    pub async fn artifact_read(
        &self,
        session: &str,
        hash: &str,
        offset: u32,
        length: u32,
    ) -> AppResult<ArtifactChunk> {
        self.artifact_metadata(session, hash).await?;
        if length == 0 || length > 32768 {
            return Err(error("分块长度须为1至32768字节"));
        }
        let bytes = self.store.blob(hash).await?;
        let start = offset as usize;
        if start > bytes.len() {
            return Err(error("偏移超过附件长度"));
        }
        let mut end = (start + length as usize).min(bytes.len());
        let (encoding, content) = if is_text(&bytes) {
            let text = std::str::from_utf8(&bytes).map_err(error)?;
            if !text.is_char_boundary(start) {
                return Err(error("偏移不在UTF-8字符边界"));
            }
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == start && start < bytes.len() {
                return Err(error("分块长度不足以容纳下一个UTF-8字符"));
            }
            ("utf8", text[start..end].to_owned())
        } else {
            ("base64", STANDARD.encode(&bytes[start..end]))
        };
        Ok(ArtifactChunk {
            encoding: encoding.into(),
            content,
            next_offset: (end < bytes.len()).then_some(end as u32),
            total_size: bytes.len() as u32,
        })
    }
    pub async fn artifact_save(&self, session: &str, hash: &str, path: &Path) -> AppResult<()> {
        self.artifact_metadata(session, hash).await?;
        let bytes = self.store.blob(hash).await?;
        tokio::fs::write(path, bytes).await.map_err(error)
    }
}
