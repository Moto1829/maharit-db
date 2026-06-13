//! maharit-viz CLI エントリーポイント
//!
//! Usage:
//!   maharit-viz [--bind <HOST:PORT>] [--server <HOST:PORT>] [--assets <DIR>]
//!
//! デフォルト:
//!   --bind   0.0.0.0:8080
//!   --server 127.0.0.1:7687
//!   --assets crates/maharit-viz/assets

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use maharit_viz::web::{VizConfig, serve};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let mut bind: Option<String> = None;
    let mut server: Option<String> = None;
    let mut assets: Option<String> = None;

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
        // バイナリ実行時のデフォルトは 0.0.0.0:8080（Docker 等から接続できるように）
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
maharit-viz [--bind <HOST:PORT>] [--server <HOST:PORT>] [--assets <DIR>]\n\n\
OPTIONS:\n    \
--bind, -b    <HOST:PORT>   HTTP bind address (default: 0.0.0.0:8080)\n    \
--server, -s  <HOST:PORT>   maharit-server TCP address (default: 127.0.0.1:7687)\n    \
--assets, -a  <DIR>         Static assets directory (default: built-in path)\n    \
--help, -h                  Print this help message"
    );
}
