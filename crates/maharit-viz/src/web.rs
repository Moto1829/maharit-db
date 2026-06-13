//! maharit-viz Web アプリケーション
//!
//! maharit-server とは別プロセスとして動作する HTTP サーバー。
//! ブラウザからクエリを受け付け、maharit-client 経由で結果を返す。
//!
//! ```text
//! ブラウザ → maharit-viz (HTTP:8080) → maharit-server (TCP:7687)
//! ```

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use maharit_client::Client;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

/// Web サーバーの設定
#[derive(Debug, Clone)]
pub struct VizConfig {
    /// HTTP サーバーのバインドアドレス（例: `0.0.0.0:8080`）
    pub bind_address: SocketAddr,
    /// maharit-server (TCP) のアドレス（例: `127.0.0.1:7687`）
    pub server_addr: String,
    /// 静的アセットを配信するディレクトリ
    pub assets_dir: PathBuf,
}

impl Default for VizConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".parse().expect("valid default address"),
            server_addr: "127.0.0.1:7687".to_string(),
            assets_dir: default_assets_dir(),
        }
    }
}

fn default_assets_dir() -> PathBuf {
    // 開発時のデフォルト位置: crates/maharit-viz/assets
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("assets");
    p
}

/// アプリケーション共有状態
#[derive(Clone)]
struct AppState {
    server_addr: Arc<String>,
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
}

/// Web アプリの Router を組み立てる（テスト用に State を共有可能）
pub fn build_router(config: &VizConfig) -> Router {
    let state = AppState {
        server_addr: Arc::new(config.server_addr.clone()),
    };

    let static_files = ServeDir::new(&config.assets_dir).append_index_html_on_directories(true);

    Router::new()
        .route("/api/query", post(query_handler))
        .route("/api/info", get(info_handler))
        .route("/api/health", get(health_handler))
        .fallback_service(static_files)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Web サーバーを起動する
pub async fn serve(config: VizConfig) -> Result<(), std::io::Error> {
    let router = build_router(&config);
    let listener = tokio::net::TcpListener::bind(config.bind_address).await?;
    tracing::info!(
        bind = %config.bind_address,
        backend = %config.server_addr,
        assets = %config.assets_dir.display(),
        "maharit-viz listening"
    );
    axum::serve(listener, router).await?;
    Ok(())
}

async fn query_handler(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!(ErrorResponse {
                error: "query is empty".to_string()
            })),
        );
    }

    let started = Instant::now();
    let result = run_query(&state.server_addr, &query).await;
    let elapsed_ms = started.elapsed().as_millis();

    match result {
        Ok(rows) => {
            let (columns, normalized) = build_columns_and_rows(rows);
            let body = QueryResponse {
                columns,
                rows: normalized,
                elapsed_ms,
            };
            (StatusCode::OK, Json(serde_json::to_value(body).unwrap()))
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!(ErrorResponse {
                error: err.to_string()
            })),
        ),
    }
}

async fn info_handler(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        server_addr: state.server_addr.as_str().to_string(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn run_query(
    server_addr: &str,
    query: &str,
) -> Result<Vec<std::collections::HashMap<String, String>>, VizError> {
    let mut client = Client::connect(server_addr)
        .await
        .map_err(|e| VizError::Backend(format!("connect: {e}")))?;
    let result = client
        .query(query)
        .await
        .map_err(|e| VizError::Backend(e.to_string()))?;
    // 接続クローズはエラーを無視（自動切断でも問題ない）
    let _ = client.disconnect().await;
    Ok(result.rows)
}

/// `Vec<HashMap<String,String>>` を JSON 用に正規化する。
/// - columns: 全行のキーを和集合した昇順リスト
/// - rows: serde_json::Map に変換し、欠損キーは null として補う
fn build_columns_and_rows(
    rows: Vec<std::collections::HashMap<String, String>>,
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
        .map(|row| {
            let mut m = serde_json::Map::with_capacity(cols.len());
            for c in &cols {
                let v = row
                    .get(c)
                    .map(|s| serde_json::Value::String(s.clone()))
                    .unwrap_or(serde_json::Value::Null);
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
        let row1: HashMap<String, String> = [
            ("n.name".to_string(), "Alice".to_string()),
            ("n.age".to_string(), "30".to_string()),
        ]
        .into_iter()
        .collect();
        let row2: HashMap<String, String> = [
            ("n.name".to_string(), "Bob".to_string()),
            ("n.city".to_string(), "Tokyo".to_string()),
        ]
        .into_iter()
        .collect();

        let (cols, rows) = build_columns_and_rows(vec![row1, row2]);

        assert_eq!(cols, vec!["n.age", "n.city", "n.name"]);
        assert_eq!(rows.len(), 2);
        // 欠損キーは null 補填
        assert_eq!(rows[0].get("n.city"), Some(&serde_json::Value::Null));
        assert_eq!(rows[1].get("n.age"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn build_columns_returns_empty_for_no_rows() {
        let (cols, rows) = build_columns_and_rows(vec![]);
        assert!(cols.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn default_config_uses_local_addresses() {
        let cfg = VizConfig::default();
        assert_eq!(cfg.bind_address.port(), 8080);
        assert!(cfg.server_addr.contains("7687"));
    }
}
