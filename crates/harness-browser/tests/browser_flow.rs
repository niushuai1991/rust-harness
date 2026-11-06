//! 集成测试：本地 HTTP 页面 + 真实系统 Edge/Chrome headless 全链路。
//! 环境未安装 Edge/Chrome 时打印原因并跳过（不视为失败）。
use harness_browser::backend::{BrowserBackend, CdpBackend};
use harness_browser::command::{BrowserCommand, SnapshotView};

const PAGE: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>浏览器集成测试页</title></head><body>
<h1>测试页</h1>
<input id="username" placeholder="用户名">
<button id="greet" onclick="greet()">打招呼</button>
<p id="result">等待操作</p>
<script>
function greet() {
  const value = document.getElementById('username').value;
  document.getElementById('result').textContent = value ? '你好，' + value : '请输入用户名';
}
</script>
</body></html>"#;

/// 起一个只服务静态测试页的极简 HTTP 服务，返回基地址。
async fn spawn_test_page() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(handle(socket));
        }
    });
    format!("http://127.0.0.1:{port}/")
}

async fn handle(mut socket: tokio::net::TcpStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = [0u8; 4096];
    let mut request = String::new();
    loop {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
                if request.contains("\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let path = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let (status, body) = if path.starts_with("/favicon") {
        ("404 Not Found", String::new())
    } else {
        ("200 OK", PAGE.to_string())
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

async fn element_snapshot(backend: &CdpBackend) -> SnapshotView {
    let outcome = backend
        .execute(BrowserCommand::Snapshot)
        .await
        .expect("快照失败");
    outcome.snapshot.expect("快照缺失")
}

#[tokio::test]
async fn full_flow_against_real_headless_browser() {
    if let Err(e) = harness_browser::launcher::find_executable(None) {
        eprintln!("跳过：{e}");
        return;
    }
    let base = spawn_test_page().await;
    let dir = tempfile::tempdir().unwrap();
    let backend = CdpBackend::new(dir.path().to_path_buf());

    let navigated = backend
        .execute(BrowserCommand::Navigate { url: base })
        .await
        .expect("导航失败");
    assert!(
        navigated.summary.contains("浏览器集成测试页"),
        "导航结果：{}",
        navigated.summary
    );

    let snapshot = element_snapshot(&backend).await;
    assert!(!snapshot.elements.is_empty(), "快照元素为空");
    let field = snapshot
        .elements
        .iter()
        .find(|e| e.name.contains("用户名"))
        .unwrap_or_else(|| panic!("未找到输入框：{:?}", snapshot.elements))
        .r#ref
        .clone();
    let button = snapshot
        .elements
        .iter()
        .find(|e| e.name.contains("打招呼") && e.role == "button")
        .unwrap_or_else(|| panic!("未找到按钮：{:?}", snapshot.elements))
        .r#ref
        .clone();

    let typed = backend
        .execute(BrowserCommand::Type {
            r#ref: field,
            text: "张三".into(),
            submit: false,
        })
        .await
        .expect("输入失败");
    assert!(typed.snapshot.is_some());

    let clicked = backend
        .execute(BrowserCommand::Click { r#ref: button })
        .await
        .expect("点击失败");
    assert!(clicked.snapshot.is_some());

    let evaluated = backend
        .execute(BrowserCommand::Evaluate {
            expression: "document.getElementById('result').textContent".into(),
        })
        .await
        .expect("评估失败");
    assert_eq!(
        evaluated.value,
        Some(serde_json::json!("你好，张三")),
        "点击按钮后的页面状态与预期不符"
    );

    let shot = backend
        .execute(BrowserCommand::Screenshot { full_page: true })
        .await
        .expect("截图失败");
    let info = shot.screenshot.expect("截图信息缺失");
    assert!(
        std::path::Path::new(&info.path).exists(),
        "截图文件不存在：{}",
        info.path
    );
    assert!(info.width > 0 && info.height > 0);

    backend
        .execute(BrowserCommand::Close)
        .await
        .expect("关闭失败");
}
