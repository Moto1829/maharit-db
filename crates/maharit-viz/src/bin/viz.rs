//! maharit-viz CLI エントリーポイント
//!
//! Usage:
//!   maharit-viz [--bind <HOST:PORT>] [--server <HOST:PORT>] [--assets <DIR>]
//!               [--auth] [--tls-cert <PATH> --tls-key <PATH>]
//!
//! デフォルト:
//!   --bind   0.0.0.0:8080
//!   --server 127.0.0.1:7687
//!   --assets crates/maharit-viz/assets
//!   --auth   無効（環境変数 MAHARIT_VIZ_AUTH=true で有効化）
//!   TLS      無効（--tls-cert + --tls-key の両方指定で有効化、
//!            または MAHARIT_VIZ_TLS_CERT + MAHARIT_VIZ_TLS_KEY）

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use maharit_viz::web::{TlsConfig, VizConfig, serve};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let mut bind: Option<String> = None;
    let mut server: Option<String> = None;
    let mut assets: Option<String> = None;
    let mut auth_flag: bool = false;
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" | "-b" => {
                if let Some(v) = args.get(i + 1) {
                    bind = Some(v.clone());
                    i += 1;
                }
            }
            "--server" | "-s" => {
                if let Some(v) = args.get(i + 1) {
                    server = Some(v.clone());
                    i += 1;
                }
            }
            "--assets" | "-a" => {
                if let Some(v) = args.get(i + 1) {
                    assets = Some(v.clone());
                    i += 1;
                }
            }
            "--auth" => {
                auth_flag = true;
            }
            "--tls-cert" => {
                if let Some(v) = args.get(i + 1) {
                    tls_cert = Some(v.clone());
                    i += 1;
                }
            }
            "--tls-key" => {
                if let Some(v) = args.get(i + 1) {
                    tls_key = Some(v.clone());
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                print_help();
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let mut config = VizConfig::default();

    if let Some(bind) = bind {
        match bind.parse::<SocketAddr>() {
            Ok(addr) => config.bind_address = addr,
            Err(e) => {
                eprintln!("invalid --bind address '{bind}': {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        config.bind_address = "0.0.0.0:8080"
            .parse()
            .expect("static default bind address must parse");
    }

    if let Some(server) = server {
        config.server_addr = server;
    }
    if let Some(assets) = assets {
        config.assets_dir = PathBuf::from(assets);
    }

    // 認証: CLI フラグ優先、なければ環境変数
    config.require_auth = if auth_flag {
        true
    } else {
        match std::env::var("MAHARIT_VIZ_AUTH").ok().as_deref() {
            Some("1") | Some("true") | Some("TRUE") | Some("yes") => true,
            _ => false,
        }
    };

    // TLS: CLI フラグ優先、なければ環境変数。
    // 両方指定があったときだけ有効化、片方だけはエラー。
    let cert = tls_cert.or_else(|| std::env::var("MAHARIT_VIZ_TLS_CERT").ok());
    let key = tls_key.or_else(|| std::env::var("MAHARIT_VIZ_TLS_KEY").ok());
    config.tls = match (cert, key) {
        (Some(c), Some(k)) => Some(TlsConfig {
            cert_path: PathBuf::from(c),
            key_path: PathBuf::from(k),
        }),
        (None, None) => None,
        (Some(_), None) => {
            eprintln!("error: --tls-cert is set but --tls-key is missing");
            return ExitCode::from(2);
        }
        (None, Some(_)) => {
            eprintln!("error: --tls-key is set but --tls-cert is missing");
            return ExitCode::from(2);
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = runtime.block_on(serve(config)) {
        eprintln!("maharit-viz server error: {e}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn print_help() {
    println!(
        "maharit-viz - MaharitDB visualization web server\n\n\
USAGE:\n    \
maharit-viz [--bind <HOST:PORT>] [--server <HOST:PORT>] [--assets <DIR>]\n               \
[--auth] [--tls-cert <PATH> --tls-key <PATH>]\n\n\
OPTIONS:\n    \
--bind, -b    <HOST:PORT>   HTTP bind address (default: 0.0.0.0:8080)\n    \
--server, -s  <HOST:PORT>   maharit-server TCP address (default: 127.0.0.1:7687)\n    \
--assets, -a  <DIR>         Static assets directory (default: built-in path)\n    \
--auth                      Require login (default: disabled). Env: MAHARIT_VIZ_AUTH=true\n    \
--tls-cert    <PATH>        TLS certificate (PEM). Env: MAHARIT_VIZ_TLS_CERT\n    \
--tls-key     <PATH>        TLS private key  (PEM). Env: MAHARIT_VIZ_TLS_KEY\n    \
--help, -h                  Print this help message"
    );
}
