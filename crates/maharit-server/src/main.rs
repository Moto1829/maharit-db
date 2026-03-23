pub mod audit;
pub mod auth;
pub mod coordinator;
pub mod http_server;
pub mod logging;
pub mod metrics;
mod repl;
pub mod replication;
pub mod tcp_server;
pub mod tls;
pub mod tracing_setup;

use repl::Repl;
use tcp_server::{ServerConfig, TcpServer};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "server" => run_server(&args[2..]),
            "backup" => run_backup(&args[2..]),
            "restore" => run_restore(&args[2..]),
            "admin" => run_admin(&args[2..]),
            "--help" | "help" => print_main_help(),
            _ => run_repl(),
        }
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
    use crate::coordinator::{CoordinatorConfig, ShardCoordinatorServer};
    use crate::replication::{
        FollowerReplicationManager, LeaderReplicationManager, NodeRole, ReplicationConfig,
    };
    use maharit_cluster::{ClusterConfig, ShardConfig};
    use maharit_storage::PersistentStorage;
    use std::sync::Arc;

    let mut config = ServerConfig::default();

    // Replication options
    let mut replication_role: Option<String> = None;
    let mut replication_bind: Option<String> = None;
    let mut leader_addr: Option<String> = None;

    // Persistence option
    let mut data_path: Option<String> = None;

    // Sharding options
    let mut shard_mode = false;
    let mut shard_id: Option<u32> = None;
    let mut coordinator_mode = false;
    let mut coordinator_port: Option<String> = None;
    let mut shard_addrs: Vec<String> = Vec::new();
    let mut sharding_strategy = "hash".to_string();

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
            "--data" | "-d" => {
                if i + 1 < args.len() {
                    data_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--replication-role" => {
                if i + 1 < args.len() {
                    replication_role = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--replication-bind" => {
                if i + 1 < args.len() {
                    replication_bind = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--leader-addr" => {
                if i + 1 < args.len() {
                    leader_addr = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--shard" => {
                shard_mode = true;
            }
            "--shard-id" => {
                if i + 1 < args.len() {
                    if let Ok(id) = args[i + 1].parse::<u32>() {
                        shard_id = Some(id);
                    }
                    i += 1;
                }
            }
            "--coordinator" => {
                coordinator_mode = true;
            }
            "--coordinator-port" => {
                if i + 1 < args.len() {
                    coordinator_port = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--shards" => {
                if i + 1 < args.len() {
                    // Comma-separated list of shard addresses
                    shard_addrs = args[i + 1]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    i += 1;
                }
            }
            "--strategy" => {
                if i + 1 < args.len() {
                    sharding_strategy = args[i + 1].clone();
                    i += 1;
                }
            }
            "--config" => {
                // Config file path (future use; skip for now)
                if i + 1 < args.len() {
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

    // ── Coordinator mode: no local graph needed ───────────────────────────────
    if coordinator_mode {
        let coord_bind = {
            let host = config.bind_address.split(':').next().unwrap_or("127.0.0.1");
            let port = coordinator_port.as_deref().unwrap_or("7690");
            format!("{}:{}", host, port)
        };

        let shards: Vec<ShardConfig> = shard_addrs
            .iter()
            .enumerate()
            .map(|(idx, addr)| ShardConfig {
                id: idx as u32,
                address: addr.clone(),
            })
            .collect();

        if shards.is_empty() {
            eprintln!("Error: --shards <ADDR,...> is required for coordinator mode");
            std::process::exit(1);
        }

        let cluster_config = ClusterConfig {
            enabled: true,
            strategy: sharding_strategy,
            shards,
            replication_factor: 1,
        };

        let coord_config = CoordinatorConfig {
            bind_address: coord_bind,
            ..CoordinatorConfig::default()
        };

        let srv = Arc::new(ShardCoordinatorServer::new(coord_config, cluster_config));
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        rt.block_on(async move {
            if let Err(e) = srv.start().await {
                eprintln!("Coordinator error: {}", e);
                std::process::exit(1);
            }
        });
        return;
    }

    // ── Shard mode: log shard ID and proceed as normal TcpServer ─────────────
    if shard_mode {
        let id = shard_id.unwrap_or(0);
        println!("Starting as shard node (shard-id={})", id);
    }

    // Resolve data path: --data > MAHARIT_DATA env var > default "maharit.db"
    let data_path = data_path
        .or_else(|| std::env::var("MAHARIT_DATA").ok())
        .unwrap_or_else(|| "maharit.db".to_string());

    // Load graph from file or start with empty graph
    let graph = if std::path::Path::new(&data_path).exists() {
        match PersistentStorage::load_concurrent(&data_path) {
            Ok(g) => {
                println!("Loaded database from {}", data_path);
                g
            }
            Err(e) => {
                eprintln!("Failed to load database from {}: {}", data_path, e);
                std::process::exit(1);
            }
        }
    } else {
        println!("Starting with empty database (will save to {})", data_path);
        maharit_core::ConcurrentGraph::new()
    };

    let graph_arc = Arc::new(graph);
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    match replication_role.as_deref() {
        Some("leader") => {
            let bind = replication_bind
                .unwrap_or_else(|| "127.0.0.1:7688".to_string());
            let repl_config = ReplicationConfig {
                role: NodeRole::Leader,
                replication_bind_address: bind,
                leader_address: None,
                ..Default::default()
            };
            let leader = Arc::new(LeaderReplicationManager::new(repl_config));
            let leader_clone = Arc::clone(&leader);
            let server = TcpServer::with_graph_arc(config, Arc::clone(&graph_arc))
                .with_replication(Arc::clone(&leader));
            let graph_for_signal = Arc::clone(&graph_arc);
            let path_for_signal = data_path.clone();
            rt.block_on(async move {
                spawn_shutdown_handler(graph_for_signal, path_for_signal);
                tokio::spawn(async move {
                    if let Err(e) = leader_clone.start().await {
                        eprintln!("Replication leader error: {}", e);
                    }
                });
                if let Err(e) = server.start().await {
                    eprintln!("Server error: {}", e);
                    std::process::exit(1);
                }
            });
        }
        Some("follower") => {
            let la = match leader_addr {
                Some(a) => a,
                None => {
                    eprintln!("Error: --leader-addr <ADDR> is required for follower role");
                    std::process::exit(1);
                }
            };
            let bind = replication_bind
                .unwrap_or_else(|| "127.0.0.1:7689".to_string());
            let repl_config = ReplicationConfig {
                role: NodeRole::Follower,
                replication_bind_address: bind,
                leader_address: Some(la),
                ..Default::default()
            };
            let follower = Arc::new(FollowerReplicationManager::new(repl_config));
            let follower_clone = Arc::clone(&follower);
            let server = TcpServer::with_graph_arc(config, Arc::clone(&graph_arc));
            let graph_for_signal = Arc::clone(&graph_arc);
            let path_for_signal = data_path.clone();
            rt.block_on(async move {
                spawn_shutdown_handler(graph_for_signal, path_for_signal);
                tokio::spawn(async move {
                    if let Err(e) = follower_clone.start().await {
                        eprintln!("Replication follower error: {}", e);
                    }
                });
                if let Err(e) = server.start().await {
                    eprintln!("Server error: {}", e);
                    std::process::exit(1);
                }
            });
        }
        Some(other) => {
            eprintln!("Unknown replication role: {}. Use 'leader' or 'follower'.", other);
            std::process::exit(1);
        }
        None => {
            let server = TcpServer::with_graph_arc(config, Arc::clone(&graph_arc));
            let graph_for_signal = Arc::clone(&graph_arc);
            let path_for_signal = data_path.clone();
            rt.block_on(async move {
                spawn_shutdown_handler(graph_for_signal, path_for_signal);
                if let Err(e) = server.start().await {
                    eprintln!("Server error: {}", e);
                    std::process::exit(1);
                }
            });
        }
    }
}

/// Spawn a background task that waits for SIGINT/SIGTERM, saves the database, then exits.
fn spawn_shutdown_handler(
    graph: std::sync::Arc<maharit_core::ConcurrentGraph>,
    data_path: String,
) {
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        println!("\nShutting down, saving database to {}...", data_path);
        match maharit_storage::PersistentStorage::save_concurrent(&graph, &data_path) {
            Ok(()) => println!("Database saved successfully."),
            Err(e) => eprintln!("Warning: Failed to save database: {}", e),
        }
        std::process::exit(0);
    });
}

/// Wait for Ctrl-C (SIGINT) or SIGTERM.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}

fn run_backup(args: &[String]) {
    use maharit_storage::{Backup, BackupOptions, CompressionType, PersistentStorage};

    let mut output_path = String::from("maharit_backup.db");
    let mut source_path: Option<String> = None;
    let mut compression = CompressionType::None;
    let mut description = String::new();
    let mut list_metadata = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                if i + 1 < args.len() {
                    output_path = args[i + 1].clone();
                    i += 1;
                }
            }
            "--source" | "-s" => {
                if i + 1 < args.len() {
                    source_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--compress" | "-z" => {
                compression = CompressionType::Gzip;
            }
            "--compress-zstd" => {
                compression = CompressionType::Zstd;
            }
            "--description" | "-d" => {
                if i + 1 < args.len() {
                    description = args[i + 1].clone();
                    i += 1;
                }
            }
            "--list" | "-l" => {
                list_metadata = true;
            }
            "--help" => {
                print_backup_help();
                return;
            }
            _ => {
                // Treat first positional arg as source path for --list, or output path otherwise
                if list_metadata && source_path.is_none() {
                    source_path = Some(args[i].clone());
                } else if !list_metadata {
                    output_path = args[i].clone();
                }
            }
        }
        i += 1;
    }

    // If --list, show metadata for a backup file
    if list_metadata {
        let path = source_path.unwrap_or(output_path);
        match Backup::metadata(&path) {
            Ok(meta) => {
                let compression_label = match meta.compression {
                    CompressionType::None => "none",
                    CompressionType::Gzip => "gzip",
                    CompressionType::Zstd => "zstd",
                };
                println!("Backup: {}", path);
                println!("  Version:     {}", meta.version);
                println!("  Created:     {}", format_timestamp(meta.timestamp));
                println!("  Nodes:       {}", meta.node_count);
                println!("  Edges:       {}", meta.edge_count);
                println!("  Compression: {}", compression_label);
                if !meta.description.is_empty() {
                    println!("  Description: {}", meta.description);
                }
            }
            Err(e) => {
                eprintln!("Failed to read backup metadata: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Create backup from a persistent storage file
    let graph = if let Some(ref src) = source_path {
        match PersistentStorage::load(src) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("Failed to open database: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Error: --source <DB_PATH> is required for backup");
        eprintln!("Usage: maharit backup --source <DB_PATH> --output <BACKUP_PATH>");
        std::process::exit(1);
    };

    let mut options = BackupOptions { compression, description: String::new() };
    if !description.is_empty() {
        options = options.with_description(description);
    }

    match Backup::create(&graph, &output_path, &options) {
        Ok(meta) => {
            let compression_label = match meta.compression {
                CompressionType::None => "none",
                CompressionType::Gzip => "gzip",
                CompressionType::Zstd => "zstd",
            };
            println!("Backup created successfully:");
            println!("  Output:      {}", output_path);
            println!("  Nodes:       {}", meta.node_count);
            println!("  Edges:       {}", meta.edge_count);
            println!("  Compression: {}", compression_label);
        }
        Err(e) => {
            eprintln!("Backup failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_restore(args: &[String]) {
    use maharit_storage::{Backup, PersistentStorage};

    let mut input_path: Option<String> = None;
    let mut output_path: Option<String> = None;
    let mut verify_only = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" | "-i" => {
                if i + 1 < args.len() {
                    input_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--output" | "-o" => {
                if i + 1 < args.len() {
                    output_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--verify" => {
                verify_only = true;
            }
            "--help" => {
                print_restore_help();
                return;
            }
            _ => {
                // First positional arg is the input path
                if input_path.is_none() {
                    input_path = Some(args[i].clone());
                }
            }
        }
        i += 1;
    }

    let input = match input_path {
        Some(p) => p,
        None => {
            eprintln!("Error: backup file path is required");
            eprintln!("Usage: maharit restore <BACKUP_PATH> [--output <DB_PATH>]");
            std::process::exit(1);
        }
    };

    if verify_only {
        match Backup::verify(&input) {
            Ok(true) => println!("Backup verification passed: {}", input),
            Ok(false) => {
                eprintln!("Backup verification failed: {}", input);
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Backup verification error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Restore graph from backup
    let graph = match Backup::restore(&input) {
        Ok(g) => {
            println!("Backup restored successfully:");
            println!("  Source: {}", input);
            println!("  Nodes: {}", g.node_count());
            println!("  Edges: {}", g.edge_count());
            g
        }
        Err(e) => {
            eprintln!("Restore failed: {}", e);
            std::process::exit(1);
        }
    };

    // If output path specified, save to persistent storage
    if let Some(ref out) = output_path {
        match PersistentStorage::save(&graph, out) {
            Ok(()) => println!("  Saved to: {}", out),
            Err(e) => {
                eprintln!("Failed to save restored database: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn format_timestamp(timestamp: u64) -> String {
    // Simple timestamp formatting without external deps
    let secs_per_day: u64 = 86400;
    let secs_per_hour: u64 = 3600;
    let secs_per_min: u64 = 60;

    // Days since Unix epoch
    let days = timestamp / secs_per_day;
    let remaining = timestamp % secs_per_day;
    let hours = remaining / secs_per_hour;
    let remaining = remaining % secs_per_hour;
    let minutes = remaining / secs_per_min;
    let seconds = remaining % secs_per_min;

    // Simple year/month/day calculation
    let mut year: u64 = 1970;
    let mut remaining_days = days;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month: u64 = 1;
    for &md in &month_days {
        if remaining_days < md {
            break;
        }
        remaining_days -= md;
        month += 1;
    }
    let day = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn run_admin(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: maharit admin <SUBCOMMAND>");
        eprintln!("  promote-to-leader --addr <ADDR>  Promote a follower to leader");
        std::process::exit(1);
    }

    match args[0].as_str() {
        "promote-to-leader" => run_admin_promote(&args[1..]),
        "--help" | "help" => {
            println!("MaharitDB Admin Commands");
            println!();
            println!("SUBCOMMANDS:");
            println!("  promote-to-leader --addr <ADDR>  Send PromoteToLeader to a follower");
        }
        other => {
            eprintln!("Unknown admin subcommand: {}", other);
            std::process::exit(1);
        }
    }
}

fn run_admin_promote(args: &[String]) {
    let mut addr: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" | "-a" => {
                if i + 1 < args.len() {
                    addr = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let addr = match addr {
        Some(a) => a,
        None => {
            eprintln!("Error: --addr <ADDR> is required");
            eprintln!("Usage: maharit admin promote-to-leader --addr <FOLLOWER_ADDR>");
            std::process::exit(1);
        }
    };

    use crate::replication::send_promote_to_leader;
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    match rt.block_on(send_promote_to_leader(&addr)) {
        Ok(()) => println!("PromoteToLeader sent to {}", addr),
        Err(e) => {
            eprintln!("Failed to send PromoteToLeader: {}", e);
            std::process::exit(1);
        }
    }
}

fn print_main_help() {
    println!("MaharitDB - Graph Database");
    println!();
    println!("USAGE:");
    println!("    maharit [COMMAND]");
    println!();
    println!("COMMANDS:");
    println!("    (default)    Start interactive REPL");
    println!("    server       Start TCP server");
    println!("    backup       Create a database backup");
    println!("    restore      Restore from a backup");
    println!("    admin        Administrative operations (replication failover, etc.)");
    println!("    help         Print this help message");
}

fn print_backup_help() {
    println!("MaharitDB Backup");
    println!();
    println!("USAGE:");
    println!("    maharit backup --source <DB_PATH> [OPTIONS]");
    println!("    maharit backup --list <BACKUP_PATH>");
    println!();
    println!("OPTIONS:");
    println!("    -s, --source <DB_PATH>       Database file to backup");
    println!("    -o, --output <BACKUP_PATH>   Output backup file (default: maharit_backup.db)");
    println!("    -z, --compress               Compress backup with gzip");
    println!("    -d, --description <TEXT>      Add a description to the backup");
    println!("    -l, --list                   Show metadata for a backup file");
    println!("    --help                       Print this help message");
}

fn print_restore_help() {
    println!("MaharitDB Restore");
    println!();
    println!("USAGE:");
    println!("    maharit restore <BACKUP_PATH> [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -i, --input <BACKUP_PATH>    Backup file to restore from");
    println!("    -o, --output <DB_PATH>       Save restored database to file");
    println!("    --verify                     Verify backup integrity without restoring");
    println!("    --help                       Print this help message");
}

fn print_server_help() {
    println!("MaharitDB TCP Server");
    println!();
    println!("USAGE:");
    println!("    maharit server [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -d, --data <PATH>                Database file path (default: maharit.db, env: MAHARIT_DATA)");
    println!("                                     Loaded on startup if exists; saved on shutdown.");
    println!("    -h, --host <HOST>                Host to bind to (default: 127.0.0.1)");
    println!("    -p, --port <PORT>                Port to listen on (default: 7687)");
    println!("    -c, --max-connections <N>        Maximum concurrent connections (default: 100)");
    println!("    --replication-role <ROLE>        Start as 'leader' or 'follower'");
    println!("    --replication-bind <ADDR>        Replication listen address (leader, default: 127.0.0.1:7688)");
    println!("    --leader-addr <ADDR>             Leader replication address (follower only)");
    println!("    --shard                          Start as a shard node (normal TcpServer with shard-id logged)");
    println!("    --shard-id <ID>                  Shard identifier for this node (used with --shard)");
    println!("    --coordinator                    Start as a coordinator node (fans queries out to shards)");
    println!("    --coordinator-port <PORT>        Port for the coordinator listener (default: 7690)");
    println!("    --shards <ADDR,...>              Comma-separated list of shard addresses (coordinator mode)");
    println!("    --strategy <STRATEGY>            Sharding strategy: hash|range|label (default: hash)");
    println!("    --config <PATH>                  Path to a TOML cluster config file (reserved)");
    println!("    --help                           Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("    maharit server --replication-role leader --replication-bind 0.0.0.0:7688");
    println!("    maharit server --replication-role follower --leader-addr 192.168.1.1:7688 --port 7690");
    println!("    maharit server --shard --shard-id 0 --port 7687");
    println!("    maharit server --shard --shard-id 1 --port 7688");
    println!("    maharit server --coordinator --shards 127.0.0.1:7687,127.0.0.1:7688 --coordinator-port 7690");
}
