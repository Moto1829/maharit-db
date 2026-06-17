//! maharit-viz Web アプリケーション
//!
//! maharit-server とは別プロセスとして動作する HTTP サーバー。
//! ブラウザからクエリを受け付け、maharit-client 経由で結果を返す。
//!
//! ```text
//! ブラウザ → maharit-viz (HTTP:8080) → maharit-server (TCP:7687)
//! ```
//!
//! ## 認証 (optional, default disabled)
//!
//! `VizConfig::require_auth = true` のとき、`/api/login` → HttpOnly Cookie
//! → middleware で `/api/query` 等を保護する流れに切り替わる。デフォルトは
//! 無効で、誰でも `/api/query` を叩ける（起動時に WARN ログを出す）。
//!
//! ## TLS (optional, default disabled)
//!
//! `VizConfig::tls = Some(TlsConfig { cert, key })` のとき HTTPS で listen する。
//! デフォルトは無効（起動時に WARN ログを出す）。

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
};
use axum::http::Request as HttpRequest;
use maharit_client::Client;
use serde::{Deserialize, Serialize};
use tower_cookies::{Cookie, CookieManagerLayer, Cookies};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

/// 認証 Cookie の名前
pub const SESSION_COOKIE: &str = "maharit_viz_session";

/// TLS 設定。両方とも指定が必須。
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// Web サーバーの設定
#[derive(Debug, Clone)]
pub struct VizConfig {
    /// HTTP サーバーのバインドアドレス（例: `0.0.0.0:8080`）
    pub bind_address: SocketAddr,
    /// maharit-server (TCP) のアドレス（例: `127.0.0.1:7687`）
    pub server_addr: String,
    /// 静的アセットを配信するディレクトリ
    pub assets_dir: PathBuf,
    /// 認証を必須にするかどうか。default: false（誰でも /api/query 可）
    pub require_auth: bool,
    /// TLS 設定。`Some` のときだけ HTTPS で listen。default: None
    pub tls: Option<TlsConfig>,
}

impl Default for VizConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".parse().expect("valid default address"),
            server_addr: "127.0.0.1:7687".to_string(),
            assets_dir: default_assets_dir(),
            require_auth: false,
            tls: None,
        }
    }
}

fn default_assets_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("assets");
    p
}

/// アプリケーション共有状態
#[derive(Clone)]
struct AppState {
    server_addr: Arc<String>,
    require_auth: bool,
    /// TLS 配下で動作中か（Cookie の Secure 属性に反映）
    serving_https: bool,
}

/// クエリ実行リクエスト
#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub query: String,
}

/// クエリ実行レスポンス（成功時）
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Map<String, serde_json::Value>>,
    pub elapsed_ms: u128,
}

/// エラーレスポンス
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// サーバー情報
#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub server_addr: String,
    pub version: &'static str,
    pub auth_enabled: bool,
    pub tls_enabled: bool,
}

/// ログインリクエスト
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// ログイン成功レスポンス
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub role: String,
    pub expires_at: u64,
}

/// Web アプリの Router を組み立てる
pub fn build_router(config: &VizConfig) -> Router {
    let state = AppState {
        server_addr: Arc::new(config.server_addr.clone()),
        require_auth: config.require_auth,
        serving_https: config.tls.is_some(),
    };

    let static_files = ServeDir::new(&config.assets_dir).append_index_html_on_directories(true);

    // 認証保護対象のルート（/api/query 等）
    let protected = Router::new()
        .route("/api/query", post(query_handler))
        .layer(middleware::from_fn_with_state(state.clone(), auth_gate));

    // 公開ルート（info / health / login）。auth 無効時は protected と差がないが、
    // route 単位で middleware を使い分けるために 2 段で組む。
    let mut public = Router::new()
        .route("/api/info", get(info_handler))
        .route("/api/health", get(health_handler));
    if config.require_auth {
        public = public.route("/api/login", post(login_handler));
        public = public.route("/api/logout", post(logout_handler));
    }

    Router::new()
        .merge(public)
        .merge(protected)
        .fallback_service(static_files)
        .layer(CookieManagerLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Web サーバーを起動する
pub async fn serve(config: VizConfig) -> Result<(), std::io::Error> {
    let router = build_router(&config);
    warn_about_disabled_features(&config);

    match &config.tls {
        Some(tls) => {
            tracing::info!(
                bind = %config.bind_address,
                backend = %config.server_addr,
                assets = %config.assets_dir.display(),
                "maharit-viz listening (TLS)"
            );
            let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_pem_file(
                &tls.cert_path,
                &tls.key_path,
            )
            .await
            .map_err(std::io::Error::other)?;
            axum_server::bind_rustls(config.bind_address, rustls_cfg)
                .serve(router.into_make_service())
                .await?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(config.bind_address).await?;
            tracing::info!(
                bind = %config.bind_address,
                backend = %config.server_addr,
                assets = %config.assets_dir.display(),
                "maharit-viz listening (HTTP)"
            );
            axum::serve(listener, router).await?;
        }
    }

    Ok(())
}

/// 起動時に無効化されている機能を WARN として出力する。
fn warn_about_disabled_features(config: &VizConfig) {
    if !config.require_auth {
        let msg = "maharit-viz authentication is DISABLED. /api/query is publicly accessible. Set MAHARIT_VIZ_AUTH=true (or --auth) to enable.";
        tracing::warn!(target: "maharit_viz", "{msg}");
        eprintln!("WARN: {msg}");
    }
    if config.tls.is_none() {
        let msg = "maharit-viz is serving over plain HTTP (no TLS). Set --tls-cert/--tls-key (or MAHARIT_VIZ_TLS_CERT/KEY) for production use.";
        tracing::warn!(target: "maharit_viz", "{msg}");
        eprintln!("WARN: {msg}");
    }
    if config.require_auth && config.tls.is_none() {
        let msg = "authentication credentials are transmitted over plain HTTP.";
        tracing::warn!(target: "maharit_viz", "{msg}");
        eprintln!("WARN: {msg}");
    }
}

// ── 認証 middleware ─────────────────────────────────────────────────────────

/// 認証保護ルートに使う middleware。`require_auth=false` の場合は素通し。
async fn auth_gate(
    State(state): State<AppState>,
    cookies: Cookies,
    request: HttpRequest<axum::body::Body>,
    next: Next,
) -> AxumResponse {
    if !state.require_auth {
        return next.run(request).await;
    }
    if cookies.get(SESSION_COOKIE).is_some() {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!(ErrorResponse {
            error: "authentication required".to_string()
        })),
    )
        .into_response()
}

// ── ハンドラ ────────────────────────────────────────────────────────────────

async fn login_handler(
    State(state): State<AppState>,
    cookies: Cookies,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let mut client = match Client::connect(state.server_addr.as_str()).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!(ErrorResponse {
                    error: format!("server connect: {e}")
                })),
            )
                .into_response();
        }
    };
    match client.login(&req.username, &req.password).await {
        Ok(info) => {
            let mut cookie = Cookie::new(SESSION_COOKIE, info.session_token.clone());
            cookie.set_http_only(true);
            cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
            cookie.set_path("/");
            if state.serving_https {
                cookie.set_secure(true);
            }
            cookies.add(cookie);
            let _ = client.disconnect().await;
            (
                StatusCode::OK,
                Json(serde_json::json!(LoginResponse {
                    role: info.role,
                    expires_at: info.expires_at,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!(ErrorResponse {
                error: e.to_string()
            })),
        )
            .into_response(),
    }
}

async fn logout_handler(cookies: Cookies) -> impl IntoResponse {
    let mut cookie = Cookie::new(SESSION_COOKIE, "");
    cookie.set_path("/");
    cookies.remove(cookie);
    StatusCode::NO_CONTENT
}

async fn query_handler(
    State(state): State<AppState>,
    cookies: Cookies,
    Json(req): Json<QueryRequest>,
) -> AxumResponse {
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!(ErrorResponse {
                error: "query is empty".to_string()
            })),
        )
            .into_response();
    }

    let session_token = cookies
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string());

    let started = Instant::now();
    let result = run_query(&state.server_addr, &query, session_token).await;
    let elapsed_ms = started.elapsed().as_millis();

    match result {
        Ok(rows) => {
            let (columns, normalized) = build_columns_and_rows(rows);
            let body = QueryResponse {
                columns,
                rows: normalized,
                elapsed_ms,
            };
            (StatusCode::OK, Json(serde_json::to_value(body).unwrap())).into_response()
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!(ErrorResponse {
                error: err.to_string()
            })),
        )
            .into_response(),
    }
}

async fn info_handler(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        server_addr: state.server_addr.as_str().to_string(),
        version: env!("CARGO_PKG_VERSION"),
        auth_enabled: state.require_auth,
        tls_enabled: state.serving_https,
    })
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn run_query(
    server_addr: &str,
    query: &str,
    session_token: Option<String>,
) -> Result<Vec<std::collections::HashMap<String, serde_json::Value>>, VizError> {
    let mut client = Client::connect(server_addr)
        .await
        .map_err(|e| VizError::Backend(format!("connect: {e}")))?;
    if session_token.is_some() {
        client.set_session_token(session_token);
    }
    let result = client
        .query(query)
        .await
        .map_err(|e| VizError::Backend(e.to_string()))?;
    let _ = client.disconnect().await;
    Ok(result.rows)
}

/// 各行の `HashMap<String, serde_json::Value>` をフロント向けに正規化する。
///
/// - columns: 全行のキーの和集合（昇順）
/// - rows: serde_json::Map で、欠損キーは null で補う
///
/// server 側 (Task: option C) で `Value::to_json()` を通して既に型情報を
/// 保持した JSON 値に変換されているので、viz では型変換は行わない。
fn build_columns_and_rows(
    rows: Vec<std::collections::HashMap<String, serde_json::Value>>,
) -> (
    Vec<String>,
    Vec<serde_json::Map<String, serde_json::Value>>,
) {
    let mut columns: BTreeSet<String> = BTreeSet::new();
    for row in &rows {
        for k in row.keys() {
            columns.insert(k.clone());
        }
    }
    let cols: Vec<String> = columns.into_iter().collect();

    let normalized: Vec<serde_json::Map<String, serde_json::Value>> = rows
        .into_iter()
        .map(|mut row| {
            let mut m = serde_json::Map::with_capacity(cols.len());
            for c in &cols {
                let v = row.remove(c).unwrap_or(serde_json::Value::Null);
                m.insert(c.clone(), v);
            }
            m
        })
        .collect();

    (cols, normalized)
}

#[derive(Debug, thiserror::Error)]
enum VizError {
    #[error("backend error: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn build_columns_unions_keys_across_rows() {
        let row1: HashMap<String, serde_json::Value> = [
            ("n.name".to_string(), serde_json::json!("Alice")),
            ("n.age".to_string(), serde_json::json!(30)),
        ]
        .into_iter()
        .collect();
        let row2: HashMap<String, serde_json::Value> = [
            ("n.name".to_string(), serde_json::json!("Bob")),
            ("n.city".to_string(), serde_json::json!("Tokyo")),
        ]
        .into_iter()
        .collect();

        let (cols, rows) = build_columns_and_rows(vec![row1, row2]);

        assert_eq!(cols, vec!["n.age", "n.city", "n.name"]);
        assert_eq!(rows.len(), 2);
        // 欠損キーは JSON null 補填
        assert_eq!(rows[0].get("n.city"), Some(&serde_json::Value::Null));
        assert_eq!(rows[1].get("n.age"), Some(&serde_json::Value::Null));
        // 型情報が維持されている (数値は JSON Number)
        assert!(rows[0].get("n.age").unwrap().is_number());
    }

    #[test]
    fn build_columns_returns_empty_for_no_rows() {
        let (cols, rows) = build_columns_and_rows(vec![]);
        assert!(cols.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn default_config_is_local_http_without_auth() {
        let cfg = VizConfig::default();
        assert_eq!(cfg.bind_address.port(), 8080);
        assert!(cfg.server_addr.contains("7687"));
        assert!(!cfg.require_auth, "auth must default to disabled");
        assert!(cfg.tls.is_none(), "tls must default to disabled");
    }

    #[test]
    fn build_router_compiles_for_each_combination() {
        // auth off / tls off
        let _ = build_router(&VizConfig::default());
        // auth on / tls off
        let cfg = VizConfig {
            require_auth: true,
            ..VizConfig::default()
        };
        let _ = build_router(&cfg);
    }
}
