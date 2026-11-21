//! 工具执行时限配置：缺省 120 秒、范围钳制、持久化往返与无效存储值兜底。
use harness_runtime::Runtime;
use serde_json::json;
use std::sync::Arc;

async fn runtime() -> (tempfile::TempDir, Arc<Runtime>) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path().join("host")).await.unwrap();
    (dir, runtime)
}

#[tokio::test]
async fn default_is_120_seconds() {
    let (_dir, rt) = runtime().await;
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 120);
}

#[tokio::test]
async fn roundtrip_persists_configured_limit() {
    let dir = tempfile::tempdir().unwrap();
    let host = dir.path().join("host");
    let rt = Runtime::open(host.clone()).await.unwrap();
    rt.set_tool_timeout_secs(300).await.unwrap();
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 300);
    // 重开后仍读已持久化的值：先释放首个实例的数据目录锁。
    drop(rt);
    let rt = Runtime::open(host).await.unwrap();
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 300);
}

#[tokio::test]
async fn rejects_out_of_range_values() {
    let (_dir, rt) = runtime().await;
    assert!(rt.set_tool_timeout_secs(4).await.is_err());
    assert!(rt.set_tool_timeout_secs(3601).await.is_err());
    // 非法设置不落盘，读取仍为缺省值。
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 120);
}

#[tokio::test]
async fn rejects_invalid_stored_value() {
    let (_dir, rt) = runtime().await;
    rt.store
        .set("app.tool_timeout_secs", &json!("not-a-number"))
        .await
        .unwrap();
    assert!(rt.tool_timeout_secs().await.is_err());
}

#[tokio::test]
async fn boundary_values_are_accepted() {
    let (_dir, rt) = runtime().await;
    rt.set_tool_timeout_secs(5).await.unwrap();
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 5);
    rt.set_tool_timeout_secs(3600).await.unwrap();
    assert_eq!(rt.tool_timeout_secs().await.unwrap(), 3600);
}
