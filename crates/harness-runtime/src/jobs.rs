//! 会话后台任务注册表：id → 归属会话与取消令牌。
//! 任务状态以 job.started/finished/cancel_requested 事件持久在会话流中，查询走纯事件重放；
//! 本注册表只承载内存侧取消句柄：background_run、前台转后台与停机清理共用同一入口。
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

/// 单个在跑后台任务的内存句柄。
pub struct JobEntry {
    pub session_id: String,
    pub token: CancellationToken,
}

#[derive(Default)]
pub struct JobRegistry {
    entries: std::sync::Mutex<HashMap<String, JobEntry>>,
}

impl JobRegistry {
    /// 登记一个在跑任务；同 id 重复登记以最后一次为准。
    pub fn register(&self, id: String, session_id: String, token: CancellationToken) {
        self.entries
            .lock()
            .unwrap()
            .insert(id, JobEntry { session_id, token });
    }
    /// 归属会话与取消令牌成对取出（background_cancel 校验会话归属后取消）。
    pub fn owner_token(&self, id: &str) -> Option<(String, CancellationToken)> {
        self.entries
            .lock()
            .unwrap()
            .get(id)
            .map(|e| (e.session_id.clone(), e.token.clone()))
    }
    pub fn contains(&self, id: &str) -> bool {
        self.entries.lock().unwrap().contains_key(id)
    }
    /// 任务结算后移除句柄；事件已持久化，不影响状态查询。
    pub fn remove(&self, id: &str) {
        self.entries.lock().unwrap().remove(id);
    }
    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }
    /// 取消全部在跑任务（宿主停机/卸载时调用）。
    pub fn cancel_all(&self) {
        for entry in self.entries.lock().unwrap().values() {
            entry.token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_lookup_and_remove_roundtrip() {
        let reg = JobRegistry::default();
        let token = CancellationToken::new();
        reg.register("j1".into(), "s1".into(), token.clone());
        assert!(reg.contains("j1"));
        assert!(!reg.is_empty());
        let (owner, handle) = reg.owner_token("j1").unwrap();
        assert_eq!(owner, "s1");
        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(reg.owner_token("j1").unwrap().1.is_cancelled());
        reg.remove("j1");
        assert!(reg.is_empty());
        assert!(reg.owner_token("j1").is_none());
    }

    #[test]
    fn cancel_all_cancels_every_entry() {
        let reg = JobRegistry::default();
        let (t1, t2) = (CancellationToken::new(), CancellationToken::new());
        reg.register("a".into(), "s".into(), t1.clone());
        reg.register("b".into(), "s".into(), t2.clone());
        reg.cancel_all();
        assert!(t1.is_cancelled() && t2.is_cancelled());
    }

    #[test]
    fn missing_ids_return_none() {
        let reg = JobRegistry::default();
        assert!(reg.owner_token("missing").is_none());
        assert!(!reg.contains("missing"));
        assert!(reg.is_empty());
    }
}
