//! ChatGPT 账号 OAuth（Sign in with ChatGPT）：PKCE 授权码登录、本地回环回调、
//! token 刷新与吊销。协议常量与流程参照 codex CLI（client_id 复用其注册身份）。
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use harness_contracts::{AppError, AppResult};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const ISSUER: &str = "https://auth.openai.com";
pub const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const ORIGINATOR: &str = "codex_cli_rs";
/// OpenAI 侧注册允许的本地回环回调端口（与 codex CLI 保持同步的允许列表）。
pub const CALLBACK_PORTS: [u16; 2] = [1455, 1457];
pub const CALLBACK_PATH: &str = "/auth/callback";
pub const CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// access_token 剩余有效期低于该阈值时在构建模型前主动刷新。
pub const REFRESH_WINDOW_SECS: i64 = 300;
/// 凭据管理器中的拆分存储引用：主记录与 refresh_token 分离以规避单值 2560 字节上限。
pub const CREDENTIAL_MAIN: &str = "provider/chatgpt";
pub const CREDENTIAL_REFRESH: &str = "provider/chatgpt.refresh";

// ---------------------------------------------------------------------------
// PKCE / state / 授权 URL
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// 64 字节随机 → base64url 无填充 verifier；challenge = BASE64URL(SHA256(verifier))（S256）。
pub fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 64];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = code_challenge(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// 32 字节随机 → base64url 无填充 state。
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 授权端点 URL；附加参数与 codex CLI 保持一致以匹配其注册的客户端身份。
pub fn build_authorize_url(
    issuer: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let mut url = reqwest::Url::parse(&format!("{issuer}/oauth/authorize"))
        .expect("issuer must be a valid base URL");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", ORIGINATOR);
    url.into()
}

// ---------------------------------------------------------------------------
// JWT 解析（base64url，无签名验证；仅取展示与路由所需 claim）
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JwtClaims {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub expires_at: Option<i64>,
}

pub fn parse_jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 从 id_token 提取账号信息；OpenAI 的账号 claim 位于 `https://api.openai.com/auth` 嵌套对象。
pub fn parse_chatgpt_claims(id_token: &str) -> JwtClaims {
    let mut claims = JwtClaims::default();
    let Some(payload) = parse_jwt_payload(id_token) else {
        return claims;
    };
    claims.email = payload
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("https://api.openai.com/profile")
                .and_then(|p| p.get("email"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if let Some(auth) = payload.get("https://api.openai.com/auth") {
        claims.account_id = auth
            .get("chatgpt_account_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        claims.plan_type = auth
            .get("chatgpt_plan_type")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    claims.expires_at = payload.get("exp").and_then(Value::as_i64);
    claims
}

pub fn jwt_expires_at(token: &str) -> Option<i64> {
    parse_jwt_payload(token)?.get("exp").and_then(Value::as_i64)
}

// ---------------------------------------------------------------------------
// TokenBundle 与拆分存储
// ---------------------------------------------------------------------------

/// 登录后的凭据包。refresh_token 持久化时与主记录拆开存放（见 `CREDENTIAL_*`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBundle {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    pub account_id: String,
    pub expires_at: i64,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
}

pub trait TokenStore: Send + Sync {
    fn load(&self, reference: &str) -> AppResult<Option<String>>;
    fn save(&self, reference: &str, value: &str) -> AppResult<()>;
    fn delete(&self, reference: &str) -> AppResult<()>;
}

/// 默认实现：Windows 凭据管理器（`harness-platform`）。
pub struct PlatformTokenStore;

impl TokenStore for PlatformTokenStore {
    fn load(&self, reference: &str) -> AppResult<Option<String>> {
        harness_platform::get_secret(reference)
    }
    fn save(&self, reference: &str, value: &str) -> AppResult<()> {
        harness_platform::set_secret(reference, value)
    }
    fn delete(&self, reference: &str) -> AppResult<()> {
        harness_platform::delete_secret(reference)
    }
}

/// 保存凭据包：先写 refresh 后写主记录，中断时旧主记录仍可与新 refresh 恢复。
pub fn save_bundle(store: &dyn TokenStore, bundle: &TokenBundle) -> AppResult<()> {
    if !bundle.refresh_token.is_empty() {
        store.save(CREDENTIAL_REFRESH, &bundle.refresh_token)?;
    }
    let main = json!({
        "accessToken": bundle.access_token,
        "accountId": bundle.account_id,
        "expiresAt": bundle.expires_at,
        "email": bundle.email,
        "planType": bundle.plan_type,
    });
    store.save(CREDENTIAL_MAIN, &main.to_string())
}

pub fn load_bundle(store: &dyn TokenStore) -> AppResult<Option<TokenBundle>> {
    let Some(main) = store.load(CREDENTIAL_MAIN)? else {
        return Ok(None);
    };
    let mut bundle: TokenBundle = serde_json::from_str(&main)
        .map_err(|e| AppError::new("MODEL_CREDENTIAL", format!("ChatGPT 凭据损坏：{e}")))?;
    bundle.refresh_token = store.load(CREDENTIAL_REFRESH)?.unwrap_or_default();
    Ok(Some(bundle))
}

pub fn delete_bundle(store: &dyn TokenStore) -> AppResult<()> {
    store.delete(CREDENTIAL_MAIN)?;
    store.delete(CREDENTIAL_REFRESH)
}

// ---------------------------------------------------------------------------
// token 端点：授权码交换（form）、刷新（JSON）、吊销
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IssuedTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
}

fn oauth_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30));
    if let Some(proxy) = proxy.filter(|p| !p.is_empty())
        && let Ok(proxy) = reqwest::Proxy::all(proxy)
    {
        builder = builder.proxy(proxy);
    }
    builder.build().unwrap_or_default()
}

async fn post_token(
    http: &reqwest::Client,
    issuer: &str,
    body: String,
    content_type: &str,
) -> AppResult<IssuedTokens> {
    let response = http
        .post(format!("{issuer}/oauth/token"))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .map_err(|_| AppError::new("MODEL_TRANSPORT", "无法连接 OpenAI 授权服务"))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|_| AppError::new("MODEL_HTTP", "授权服务返回了无法解析的响应"))?;
    if !status.is_success() {
        let code = value.get("error").and_then(Value::as_str).unwrap_or("");
        let message = value
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or("授权失败");
        let kind = if classify_refresh_failure(status.as_u16(), code) == RefreshFailure::Permanent {
            "MODEL_AUTH"
        } else {
            "MODEL_HTTP"
        };
        return Err(AppError::new(kind, message.to_string()));
    }
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::new("MODEL_HTTP", "授权响应缺少 access_token"))?
        .to_string();
    let id_token = value
        .get("id_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(IssuedTokens {
        id_token,
        access_token,
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// 授权码换 token（RFC 6749 §4.1.3，form 编码）。
pub async fn exchange_code(
    http: &reqwest::Client,
    issuer: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> AppResult<IssuedTokens> {
    let form = format!(
        "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
        urlencode(CLIENT_ID),
        urlencode(code),
        urlencode(redirect_uri),
        urlencode(verifier),
    );
    post_token(http, issuer, form, "application/x-www-form-urlencoded").await
}

/// 刷新 access_token（与 codex 一致使用 JSON 编码并携带 refresh_reason）。
pub async fn refresh_tokens(
    http: &reqwest::Client,
    issuer: &str,
    refresh_token: &str,
    reason: &str,
) -> AppResult<IssuedTokens> {
    post_token(
        http,
        issuer,
        json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
            "refresh_reason": reason,
        })
        .to_string(),
        "application/json",
    )
    .await
}

/// 吊销 refresh_token；失败由调用方决定是否容忍（登出时不应阻塞本地清理）。
pub async fn revoke_token(
    http: &reqwest::Client,
    issuer: &str,
    refresh_token: &str,
) -> AppResult<()> {
    let form = format!(
        "token={}&client_id={}",
        urlencode(refresh_token),
        urlencode(CLIENT_ID)
    );
    let response = http
        .post(format!("{issuer}/oauth/revoke"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await
        .map_err(|_| AppError::new("MODEL_TRANSPORT", "无法连接 OpenAI 授权服务"))?;
    if !response.status().is_success() {
        return Err(AppError::new("MODEL_HTTP", "吊销 token 失败"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshFailure {
    Permanent,
    Transient,
}

/// 刷新失败分类：refresh_token 失效类错误为永久失败（要求重新登录），其余可重试。
pub fn classify_refresh_failure(status: u16, error: &str) -> RefreshFailure {
    let permanent_codes = [
        "refresh_token_expired",
        "refresh_token_reused",
        "refresh_token_invalidated",
        "invalid_grant",
    ];
    if status == 401 || (status == 400 && permanent_codes.contains(&error)) {
        RefreshFailure::Permanent
    } else {
        RefreshFailure::Transient
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// access_token 是否需要刷新：剩余有效期不足阈值即需要。
pub fn needs_refresh(expires_at: i64, now: i64) -> bool {
    expires_at - now <= REFRESH_WINDOW_SECS
}

/// 由 token 端点响应合成凭据包：claims 取自 id_token，expires_at 取自 access_token 的 exp。
pub fn bundle_from_issued(
    issued: &IssuedTokens,
    previous_refresh: Option<&str>,
) -> AppResult<TokenBundle> {
    let claims = parse_chatgpt_claims(&issued.id_token);
    let expires_at = jwt_expires_at(&issued.access_token)
        .unwrap_or_else(|| chrono::Utc::now().timestamp() + 3600);
    let refresh_token = issued
        .refresh_token
        .clone()
        .or_else(|| previous_refresh.map(str::to_string))
        .unwrap_or_default();
    let Some(account_id) = claims.account_id else {
        return Err(AppError::new(
            "MODEL_CREDENTIAL",
            "登录响应缺少 ChatGPT 账号标识",
        ));
    };
    Ok(TokenBundle {
        access_token: issued.access_token.clone(),
        refresh_token,
        account_id,
        expires_at,
        email: claims.email,
        plan_type: claims.plan_type,
    })
}

/// 返回可直接用于请求的凭据包；临近过期时先刷新并回写凭据。
pub async fn ensure_access_token(
    store: &dyn TokenStore,
    bundle: TokenBundle,
    issuer: &str,
    now: i64,
    proxy: Option<&str>,
) -> AppResult<TokenBundle> {
    if !needs_refresh(bundle.expires_at, now) {
        return Ok(bundle);
    }
    if bundle.refresh_token.is_empty() {
        return Err(AppError::new(
            "MODEL_AUTH",
            "ChatGPT 登录已过期且缺少刷新凭据，请重新登录",
        ));
    }
    let http = oauth_client(proxy);
    let issued =
        match refresh_tokens(&http, issuer, &bundle.refresh_token, "proactive_expiry").await {
            Ok(issued) => issued,
            // 永久失败（refresh_token 失效）直接要求重新登录。
            Err(e) if e.code == "MODEL_AUTH" => return Err(e),
            // 瞬时失败不阻塞本次请求：旧 token 在 exp 前通常仍有效。
            Err(_) => return Ok(bundle),
        };
    let updated = bundle_from_issued(&issued, Some(&bundle.refresh_token))?;
    save_bundle(store, &updated)?;
    Ok(updated)
}

// ---------------------------------------------------------------------------
// 本地回环回调登录流程
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LoginOptions {
    /// 授权与 token 端点的 issuer；测试注入 mock 服务。
    pub issuer: String,
    /// 回调端口；None 依次尝试 `CALLBACK_PORTS`，Some(0) 由系统分配（测试）。
    pub port: Option<u16>,
    /// 访问授权服务的代理（供应商个人覆盖层可配）。
    pub proxy: Option<String>,
    pub store: Arc<dyn TokenStore>,
}

impl LoginOptions {
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        Self {
            issuer: ISSUER.to_string(),
            port: None,
            proxy: None,
            store,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginPhase {
    Pending,
    Completed {
        email: Option<String>,
        plan_type: Option<String>,
    },
    Failed(String),
    Cancelled,
}

pub struct LoginHandle {
    pub auth_url: String,
    pub port: u16,
    result_rx: watch::Receiver<Option<AppResult<TokenBundle>>>,
    cancel: CancellationToken,
}

impl LoginHandle {
    /// 等待登录结果；流程终止（发送端关闭）时返回取消错误。
    pub async fn wait(&self) -> AppResult<TokenBundle> {
        let mut rx = self.result_rx.clone();
        let settled = rx
            .wait_for(|v| v.is_some())
            .await
            .map_err(|_| AppError::new("CANCELLED", "登录流程已终止"))?;
        match settled.as_ref() {
            Some(Ok(bundle)) => Ok(bundle.clone()),
            Some(Err(e)) => Err(e.clone()),
            None => Err(AppError::new("CANCELLED", "登录未完成")),
        }
    }
    pub fn phase(&self) -> LoginPhase {
        match &*self.result_rx.borrow() {
            None => LoginPhase::Pending,
            Some(Ok(bundle)) => LoginPhase::Completed {
                email: bundle.email.clone(),
                plan_type: bundle.plan_type.clone(),
            },
            Some(Err(e)) if e.code == "CANCELLED" => LoginPhase::Cancelled,
            Some(Err(e)) => LoginPhase::Failed(e.message.clone()),
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// 登录回调循环的上下文；把 OAuth 参数打包为一个值以收敛后台任务签名。
struct LoginFlow {
    issuer: String,
    proxy: Option<String>,
    store: Arc<dyn TokenStore>,
    redirect_uri: String,
    pkce: Pkce,
    state: String,
}

/// 启动浏览器 OAuth 登录：绑定回环端口、生成 PKCE/state 并在后台等待回调。
/// 打开浏览器由调用方负责（CLI 用 `webbrowser`，桌面用系统 opener）。
pub async fn start_login(options: LoginOptions) -> AppResult<LoginHandle> {
    let listener = bind_listener(options.port).await?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::new("LOGIN_PORT", e.to_string()))?
        .port();
    let pkce = generate_pkce();
    let state = generate_state();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let auth_url = build_authorize_url(&options.issuer, &redirect_uri, &pkce.challenge, &state);
    let flow = LoginFlow {
        issuer: options.issuer,
        proxy: options.proxy,
        store: options.store,
        redirect_uri,
        pkce,
        state,
    };
    let (tx, rx) = watch::channel(None);
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    tokio::spawn(run_login_loop(listener, flow, tx, task_cancel));
    Ok(LoginHandle {
        auth_url,
        port,
        result_rx: rx,
        cancel,
    })
}

async fn bind_listener(port: Option<u16>) -> AppResult<TcpListener> {
    let candidates: Vec<u16> = match port {
        Some(port) => vec![port],
        None => CALLBACK_PORTS.to_vec(),
    };
    let mut last = String::new();
    for candidate in &candidates {
        match TcpListener::bind(("127.0.0.1", *candidate)).await {
            Ok(listener) => return Ok(listener),
            Err(e) => last = e.to_string(),
        }
    }
    Err(AppError::new(
        "LOGIN_PORT",
        format!(
            "本地回调端口 {} 均不可用：{last}",
            candidates
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join("、")
        ),
    ))
}

async fn run_login_loop(
    listener: TcpListener,
    flow: LoginFlow,
    tx: watch::Sender<Option<AppResult<TokenBundle>>>,
    cancel: CancellationToken,
) {
    let http = oauth_client(flow.proxy.as_deref());
    let mut result: AppResult<TokenBundle> = Err(AppError::new("CANCELLED", "登录已取消"));
    loop {
        let (mut stream, _) = tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(_) => continue,
            },
        };
        let Some(exchange) = read_http(&mut stream).await else {
            continue;
        };
        if exchange.request.path != CALLBACK_PATH {
            if exchange.request.path == "/cancel" {
                let page = failure_page("已取消登录，可关闭此页面。");
                let _ = write_response(stream, 200, &page).await;
            } else {
                let _ = write_response(stream, 404, "Not Found").await;
            }
            continue;
        }
        let params = parse_query(exchange.request.query.as_deref().unwrap_or_default());
        // state 不匹配对浏览器返回 400，且不进入 token 端点交换。
        if params.get("state").map(String::as_str) != Some(flow.state.as_str()) {
            let page = failure_page("回调校验失败：state 不匹配，请重新发起登录。");
            let _ = write_response(stream, 400, &page).await;
            result = Err(AppError::new(
                "MODEL_AUTH",
                "回调 state 校验失败，请重新发起登录",
            ));
            break;
        }
        result = match settle_callback(
            &http,
            &flow.issuer,
            &*flow.store,
            &flow.redirect_uri,
            &flow.pkce,
            &flow.state,
            &params,
        )
        .await
        {
            Ok(bundle) => {
                let _ = write_response(stream, 200, &success_page()).await;
                Ok(bundle)
            }
            Err(e) => {
                let _ = write_response(stream, 200, &failure_page(&e.message)).await;
                Err(e)
            }
        };
        break;
    }
    let _ = tx.send(Some(result));
}

async fn settle_callback(
    http: &reqwest::Client,
    issuer: &str,
    store: &dyn TokenStore,
    redirect_uri: &str,
    pkce: &Pkce,
    state: &str,
    params: &std::collections::HashMap<String, String>,
) -> AppResult<TokenBundle> {
    if params.get("state").map(String::as_str) != Some(state) {
        return Err(AppError::new(
            "MODEL_AUTH",
            "回调 state 校验失败，请重新发起登录",
        ));
    }
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| "授权被拒绝".to_string());
        let _ = error;
        return Err(AppError::new("MODEL_AUTH", description));
    }
    let code = params
        .get("code")
        .ok_or_else(|| AppError::new("MODEL_AUTH", "回调缺少授权码"))?;
    let issued = exchange_code(http, issuer, code, &pkce.verifier, redirect_uri).await?;
    let bundle = bundle_from_issued(&issued, None)?;
    save_bundle(store, &bundle)?;
    Ok(bundle)
}

#[derive(Debug)]
struct HttpRequest {
    path: String,
    query: Option<String>,
}

struct HttpExchange {
    request: HttpRequest,
    /// 请求 body；回调路径读取即消费不用其内容（mock issuer 测试依赖）。
    #[allow(dead_code)]
    body: Vec<u8>,
}

/// 读取一个 HTTP 请求（head + content-length 限定的 body）。
async fn read_http(stream: &mut tokio::net::TcpStream) -> Option<HttpExchange> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (target, None),
    };
    let content_length: usize = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Some(HttpExchange {
        request: HttpRequest { path, query },
        body,
    })
}

async fn write_response(
    mut stream: tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> AppResult<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|e| AppError::new("MODEL_TRANSPORT", e.to_string()))?;
    stream
        .write_all(body.as_bytes())
        .await
        .map_err(|e| AppError::new("MODEL_TRANSPORT", e.to_string()))?;
    stream.flush().await.ok();
    Ok(())
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (urldecode(key), urldecode(value)),
            None => (urldecode(pair), String::new()),
        })
        .collect()
}

fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn success_page() -> String {
    r#"<html><body style="font-family:system-ui,sans-serif;text-align:center;padding-top:80px;background:#fafafa"><h2>&#9989; 登录成功</h2><p>已保存 ChatGPT 账号凭据，请返回应用继续。</p></body></html>"#.to_string()
}

fn failure_page(message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<html><body style="font-family:system-ui,sans-serif;text-align:center;padding-top:80px;background:#fafafa"><h2>&#9888; 登录未完成</h2><p>{escaped}</p></body></html>"#
    )
}

// ---------------------------------------------------------------------------
// 登出
// ---------------------------------------------------------------------------

/// 吊销 refresh_token 后删除本地凭据；吊销失败不阻塞本地删除。
pub async fn logout(store: &dyn TokenStore, issuer: &str, proxy: Option<&str>) -> AppResult<()> {
    if let Ok(Some(bundle)) = load_bundle(store)
        && !bundle.refresh_token.is_empty()
    {
        let http = oauth_client(proxy);
        let _ = revoke_token(&http, issuer, &bundle.refresh_token).await;
    }
    delete_bundle(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MemoryStore(StdMutex<HashMap<String, String>>);
    impl TokenStore for MemoryStore {
        fn load(&self, reference: &str) -> AppResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(reference).cloned())
        }
        fn save(&self, reference: &str, value: &str) -> AppResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(reference.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, reference: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(reference);
            Ok(())
        }
    }

    fn bundle() -> TokenBundle {
        TokenBundle {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            account_id: "acc".into(),
            expires_at: 1000,
            email: Some("a@b.c".into()),
            plan_type: Some("plus".into()),
        }
    }

    /// 自制 alg=none JWT（header.payload.signature 三段 base64url）。
    fn fake_jwt(payload: &Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{body}.x")
    }

    #[test]
    fn pkce_verifier_shape_and_challenge_matches() {
        let pkce = generate_pkce();
        assert_eq!(
            pkce.verifier.len(),
            86,
            "64 字节 base64url 无填充为 86 字符"
        );
        assert!(
            pkce.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        );
        assert_eq!(pkce.challenge, code_challenge(&pkce.verifier));
    }

    #[test]
    fn state_is_urlsafe_43chars() {
        let state = generate_state();
        assert_eq!(state.len(), 43);
        assert_ne!(state, generate_state());
    }

    #[test]
    fn authorize_url_contains_required_params() {
        let url = build_authorize_url(
            ISSUER,
            "http://127.0.0.1:1455/auth/callback",
            "challenge-1",
            "state-1",
        );
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        for fragment in [
            "response_type=code",
            "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback",
            "scope=openid+profile+email+offline_access+api.connectors.read+api.connectors.invoke",
            "state=state-1",
            "code_challenge=challenge-1",
            "code_challenge_method=S256",
            "id_token_add_organizations=true",
            "codex_cli_simplified_flow=true",
            "originator=codex_cli_rs",
        ] {
            assert!(url.contains(fragment), "缺少 {fragment}：{url}");
        }
    }

    #[test]
    fn jwt_claims_parse_openai_auth_namespace() {
        let token = fake_jwt(&json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-1",
                "chatgpt_plan_type": "plus"
            },
            "exp": 1234i64
        }));
        let claims = parse_chatgpt_claims(&token);
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.account_id.as_deref(), Some("acc-1"));
        assert_eq!(claims.plan_type.as_deref(), Some("plus"));
        assert_eq!(claims.expires_at, Some(1234));
    }

    #[test]
    fn jwt_claims_fall_back_to_profile_email_and_tolerate_garbage() {
        let token = fake_jwt(&json!({
            "https://api.openai.com/profile": {"email": "p@example.com"},
            "https://api.openai.com/auth": {"user_id": "u1"}
        }));
        let claims = parse_chatgpt_claims(&token);
        assert_eq!(claims.email.as_deref(), Some("p@example.com"));
        assert_eq!(claims.account_id, None);
        assert_eq!(parse_chatgpt_claims("not-a-jwt"), JwtClaims::default());
    }

    #[test]
    fn bundle_split_storage_roundtrip() {
        let store = MemoryStore::default();
        save_bundle(&store, &bundle()).unwrap();
        let main = store
            .0
            .lock()
            .unwrap()
            .get(CREDENTIAL_MAIN)
            .unwrap()
            .clone();
        assert!(
            !main.contains("refresh_token") && !main.contains("refreshToken"),
            "主记录不得包含 refresh_token：{main}"
        );
        assert_eq!(
            store.0.lock().unwrap().get(CREDENTIAL_REFRESH).unwrap(),
            "rt"
        );
        let loaded = load_bundle(&store).unwrap().unwrap();
        assert_eq!(loaded, bundle());
        delete_bundle(&store).unwrap();
        assert_eq!(load_bundle(&store).unwrap(), None);
    }

    #[test]
    fn bundle_without_refresh_leaves_previous_refresh_intact() {
        let store = MemoryStore::default();
        let mut refreshed = bundle();
        refreshed.refresh_token = String::new();
        save_bundle(&store, &refreshed).unwrap();
        assert!(
            !store.0.lock().unwrap().contains_key(CREDENTIAL_REFRESH),
            "空 refresh_token 不得覆盖既有值"
        );
    }

    #[test]
    fn needs_refresh_window() {
        assert!(needs_refresh(1000, 1000 - REFRESH_WINDOW_SECS));
        assert!(!needs_refresh(1000, 1000 - REFRESH_WINDOW_SECS - 1));
    }

    #[test]
    fn classify_refresh_failure_categories() {
        use RefreshFailure::*;
        assert_eq!(classify_refresh_failure(400, "invalid_grant"), Permanent);
        assert_eq!(
            classify_refresh_failure(400, "refresh_token_expired"),
            Permanent
        );
        assert_eq!(classify_refresh_failure(401, ""), Permanent);
        assert_eq!(classify_refresh_failure(500, ""), Transient);
        assert_eq!(classify_refresh_failure(400, "slow_down"), Transient);
    }

    #[test]
    fn bundle_requires_account_id() {
        let issued = IssuedTokens {
            id_token: fake_jwt(&json!({"email": "e@x.com"})),
            access_token: fake_jwt(&json!({"exp": 9000i64})),
            refresh_token: Some("rt2".into()),
        };
        assert!(bundle_from_issued(&issued, None).is_err());
    }

    #[test]
    fn bundle_from_issued_keeps_previous_refresh_when_omitted() {
        let issued = IssuedTokens {
            id_token: fake_jwt(&json!({
                "email": "e@x.com",
                "https://api.openai.com/auth": {"chatgpt_account_id": "acc"}
            })),
            access_token: fake_jwt(&json!({"exp": 9000i64})),
            refresh_token: None,
        };
        let next = bundle_from_issued(&issued, Some("old-rt")).unwrap();
        assert_eq!(next.refresh_token, "old-rt");
        assert_eq!(next.account_id, "acc");
    }

    // ---------------- e2e：mock issuer + 回调服务器 ----------------

    type RequestLog = Arc<StdMutex<Vec<(String, Vec<u8>)>>>;

    /// 记录收到的请求并返回固定 JSON 的 mock issuer（POST /oauth/token）。
    async fn spawn_mock_issuer(response: Value) -> (u16, RequestLog) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests: RequestLog = Arc::default();
        let requests_for_task = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_task.clone();
                let response = response.clone();
                tokio::spawn(async move {
                    let Some(exchange) = read_http(&mut stream).await else {
                        return;
                    };
                    requests
                        .lock()
                        .unwrap()
                        .push((exchange.request.path, exchange.body));
                    write_response(stream, 200, &response.to_string())
                        .await
                        .ok();
                });
            }
        });
        (port, requests)
    }

    fn token_response() -> Value {
        json!({
            "access_token": fake_jwt(&json!({"exp": 9000i64})),
            "id_token": fake_jwt(&json!({
                "email": "user@example.com",
                "https://api.openai.com/auth": {"chatgpt_account_id": "acc-77", "chatgpt_plan_type": "pro"}
            })),
            "refresh_token": "rt-rotated"
        })
    }

    /// 模拟浏览器回调请求。
    async fn browser_callback(port: u16, query: &str) -> reqwest::Response {
        reqwest::get(format!("http://127.0.0.1:{port}{CALLBACK_PATH}?{query}"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn callback_flow_exchanges_and_persists_bundle() {
        let (issuer_port, requests) = spawn_mock_issuer(token_response()).await;
        let store = Arc::new(MemoryStore::default());
        let handle = start_login(LoginOptions {
            issuer: format!("http://127.0.0.1:{issuer_port}"),
            proxy: None,
            port: Some(0),
            store: store.clone(),
        })
        .await
        .unwrap();
        let state = extract_query_param(&handle.auth_url, "state").unwrap();
        let response = browser_callback(handle.port, &format!("code=xyz&state={state}")).await;
        assert!(response.status().is_success());
        let bundle = tokio::time::timeout(std::time::Duration::from_secs(5), handle.wait())
            .await
            .expect("登录应在超时前完成")
            .unwrap();
        assert_eq!(bundle.account_id, "acc-77");
        assert_eq!(bundle.refresh_token, "rt-rotated");
        assert_eq!(bundle.email.as_deref(), Some("user@example.com"));
        assert_eq!(bundle.plan_type.as_deref(), Some("pro"));
        // 保存的凭据包与拆分存储一致。
        let saved = load_bundle(&*store).unwrap().unwrap();
        assert_eq!(saved, bundle);
        // 授权码交换使用 form 编码并携带 PKCE verifier。
        let log = requests.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "/oauth/token");
        let body = String::from_utf8(log[0].1.clone()).unwrap();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("code_verifier="));
        assert!(body.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    }

    #[tokio::test]
    async fn callback_state_mismatch_rejects_and_fails_login() {
        let (issuer_port, _requests) = spawn_mock_issuer(token_response()).await;
        let store = Arc::new(MemoryStore::default());
        let handle = start_login(LoginOptions {
            issuer: format!("http://127.0.0.1:{issuer_port}"),
            proxy: None,
            port: Some(0),
            store,
        })
        .await
        .unwrap();
        let response = browser_callback(handle.port, "code=xyz&state=wrong").await;
        assert_eq!(response.status().as_u16(), 400);
        let err = handle.wait().await.unwrap_err();
        assert_eq!(err.code, "MODEL_AUTH");
        assert_eq!(
            handle.phase(),
            LoginPhase::Failed("回调 state 校验失败，请重新发起登录".into())
        );
    }

    #[tokio::test]
    async fn callback_error_param_fails_login() {
        let (issuer_port, requests) = spawn_mock_issuer(token_response()).await;
        let store = Arc::new(MemoryStore::default());
        let handle = start_login(LoginOptions {
            issuer: format!("http://127.0.0.1:{issuer_port}"),
            proxy: None,
            port: Some(0),
            store: store.clone(),
        })
        .await
        .unwrap();
        let state = extract_query_param(&handle.auth_url, "state").unwrap();
        let response = browser_callback(
            handle.port,
            &format!("error=access_denied&error_description=User+denied&state={state}"),
        )
        .await;
        assert!(response.status().is_success());
        let err = handle.wait().await.unwrap_err();
        assert_eq!(err.message, "User denied");
        assert!(load_bundle(&*store).unwrap().is_none(), "失败不得写凭据");
        assert!(
            requests.lock().unwrap().is_empty(),
            "失败不得调用 token 端点"
        );
    }

    #[tokio::test]
    async fn cancel_terminates_pending_login() {
        let (issuer_port, _requests) = spawn_mock_issuer(token_response()).await;
        let handle = start_login(LoginOptions {
            issuer: format!("http://127.0.0.1:{issuer_port}"),
            proxy: None,
            port: Some(0),
            store: Arc::new(MemoryStore::default()),
        })
        .await
        .unwrap();
        assert_eq!(handle.phase(), LoginPhase::Pending);
        handle.cancel();
        let err = handle.wait().await.unwrap_err();
        assert_eq!(err.code, "CANCELLED");
        assert_eq!(handle.phase(), LoginPhase::Cancelled);
    }

    #[tokio::test]
    async fn refresh_flow_rotates_tokens_and_persists() {
        let (issuer_port, requests) = spawn_mock_issuer(token_response()).await;
        let issuer = format!("http://127.0.0.1:{issuer_port}");
        let store = MemoryStore::default();
        let mut current = bundle();
        current.expires_at = chrono::Utc::now().timestamp() + 60; // 5 分钟窗口内
        save_bundle(&store, &current).unwrap();
        let refreshed = ensure_access_token(
            &store,
            current,
            &issuer,
            chrono::Utc::now().timestamp(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(refreshed.refresh_token, "rt-rotated");
        assert_eq!(refreshed.access_token, token_response()["access_token"]);
        let saved = load_bundle(&store).unwrap().unwrap();
        assert_eq!(saved, refreshed);
        let log = requests.lock().unwrap();
        assert_eq!(log.len(), 1);
        let body = String::from_utf8(log[0].1.clone()).unwrap();
        let payload: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["grant_type"], "refresh_token");
        assert_eq!(payload["refresh_token"], "rt");
        assert_eq!(payload["refresh_reason"], "proactive_expiry");
    }

    #[tokio::test]
    async fn ensure_access_token_skips_fresh_token() {
        let (issuer_port, requests) = spawn_mock_issuer(token_response()).await;
        let issuer = format!("http://127.0.0.1:{issuer_port}");
        let store = MemoryStore::default();
        let fresh = bundle(); // expires_at=1000, now 远小于窗口阈值
        let kept = ensure_access_token(&store, fresh, &issuer, 0, None)
            .await
            .unwrap();
        assert_eq!(kept.access_token, "at");
        assert!(requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn permanent_refresh_failure_requires_relogin() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // 必须先读完请求再响应，否则未读 body 触发 RST 使客户端读取失败。
            let _ = read_http(&mut stream).await;
            let _ = write_response(
                stream,
                400,
                r#"{"error":"refresh_token_expired","error_description":"expired"}"#,
            )
            .await;
        });
        let store = MemoryStore::default();
        let mut current = bundle();
        current.expires_at = 0;
        let issuer = format!("http://127.0.0.1:{port}");
        let err = ensure_access_token(&store, current, &issuer, 100, None)
            .await
            .err()
            .unwrap();
        assert_eq!(err.code, "MODEL_AUTH");
    }

    fn extract_query_param(url: &str, key: &str) -> Option<String> {
        let query = url.split_once('?')?.1;
        parse_query(query).get(key).cloned()
    }
}
