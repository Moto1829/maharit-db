pub mod audit;
pub mod auth;
pub mod http_server;
pub mod logging;
pub mod metrics;
mod repl;
pub mod replication;
pub mod tcp_server;
pub mod tls;

use repl::Repl;
use tcp_server::{ServerConfig, TcpServer};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "server" => run_server(&args[2..]),
            "backup" => run_backup(&args[2..]),
            "restore" => run_restore(&args[2..]),
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

fn run_backup(args: &[String]) {
    use maharit_storage::{Backup, BackupOptions, PersistentStorage};

    let mut output_path = String::from("maharit_backup.db");
    let mut source_path: Option<String> = None;
    let mut compressed = false;
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
                compressed = true;
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
                println!("Backup: {}", path);
                println!("  Version:     {}", meta.version);
                println!("  Created:     {}", format_timestamp(meta.timestamp));
                println!("  Nodes:       {}", meta.node_count);
                println!("  Edges:       {}", meta.edge_count);
                println!("  Compressed:  {}", meta.compressed);
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

    let mut options = if compressed {
        BackupOptions::compressed()
    } else {
        BackupOptions::default()
    };
    if !description.is_empty() {
        options = options.with_description(description);
    }

    match Backup::create(&graph, &output_path, &options) {
        Ok(meta) => {
            println!("Backup created successfully:");
            println!("  Output:     {}", output_path);
            println!("  Nodes:      {}", meta.node_count);
            println!("  Edges:      {}", meta.edge_count);
            println!("  Compressed: {}", meta.compressed);
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
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
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
    println!("    -h, --host <HOST>            Host to bind to (default: 127.0.0.1)");
    println!("    -p, --port <PORT>            Port to listen on (default: 7687)");
    println!("    -c, --max-connections <N>    Maximum concurrent connections (default: 100)");
    println!("    --help                       Print this help message");
}
