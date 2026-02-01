mod repl;
pub mod tcp_server;

use repl::Repl;
use tcp_server::{ServerConfig, TcpServer};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "server" {
        // Run in server mode
        run_server(&args[2..]);
    } else {
        // Run in REPL mode
        run_repl();
    }
}

fn run_repl() {
    match Repl::new() {
        Ok(mut repl) => {
            if let Err(e) = repl.run() {
                eprintln!("REPL error: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to initialize REPL: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_server(args: &[String]) {
    let mut config = ServerConfig::default();

    // Parse arguments
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" | "-h" => {
                if i + 1 < args.len() {
                    let host = &args[i + 1];
                    let port = config.bind_address.split(':').nth(1).unwrap_or("7687");
                    config.bind_address = format!("{}:{}", host, port);
                    i += 1;
                }
            }
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    let host = config.bind_address.split(':').next().unwrap_or("127.0.0.1");
                    config.bind_address = format!("{}:{}", host, &args[i + 1]);
                    i += 1;
                }
            }
            "--max-connections" | "-c" => {
                if i + 1 < args.len() {
                    if let Ok(n) = args[i + 1].parse() {
                        config.max_connections = n;
                    }
                    i += 1;
                }
            }
            "--help" => {
                print_server_help();
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let server = TcpServer::new(config);

    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    rt.block_on(async {
        if let Err(e) = server.start().await {
            eprintln!("Server error: {}", e);
            std::process::exit(1);
        }
    });
}

fn print_server_help() {
    println!("MaharitDB TCP Server");
    println!();
    println!("USAGE:");
    println!("    maharit server [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --host <HOST>            Host to bind to (default: 127.0.0.1)");
    println!("    -p, --port <PORT>            Port to listen on (default: 7687)");
    println!("    -c, --max-connections <N>    Maximum concurrent connections (default: 100)");
    println!("    --help                       Print this help message");
}
