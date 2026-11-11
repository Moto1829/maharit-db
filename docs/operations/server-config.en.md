---
title: Server configuration
parent: Server and operations
nav_order: 1
source_commit: 6e4c993e
---

# Server configuration

## Persistence

The MaharitDB server **always persists to a file**. As with SQLite, you give the path of a data file at startup.

- If the file exists: the existing data is loaded
- If the file does not exist: a new one is created
- Write queries: changes are appended to the WAL (`<data file>.wal`) before the reply
- On SIGINT / SIGTERM: the data file is saved, the WAL is emptied, and the server shuts down

> **Note**: the server has no in-memory-only mode. For in-memory use, use `Graph::new()` from `maharit-core` directly.

### The WAL and crash recovery

The data file (snapshot) holds the whole graph and is rewritten at shutdown and at checkpoints. Changes in between are appended to the WAL for each write query.

At startup the server loads the data file and replays the WAL from the beginning, so even if the process is killed (`kill -9`, OOM, power loss, and so on), **acknowledged writes are not lost**. If the last record was left half-written (a write that crashed before the reply), it is truncated at startup.

`--wal` chooses how the WAL is written.

| Mode | Behavior | Survives |
|--------|------|-----------|
| `fsync` (default) | Append and fsync before replying | Killed processes, OS crashes, power loss |
| `os` | Append without fsync (left to the OS cache) | Killed processes only |
| `off` | No WAL (saves only at shutdown and checkpoints) | Nothing (changes since the last save are lost) |

In `fsync` mode, the fsyncs of writes that arrive together are combined into one (group commit). Write throughput is still bounded by the disk's fsync performance; on macOS (APFS), expect a few hundred writes per second per connection. For bulk loads, combine writes into one query with `UNWIND` (one query = one fsync) or use `--wal os` temporarily.

**Checkpoints**: when the WAL exceeds `--checkpoint-mb` (default 64MB), writes pause while the data file is saved and the WAL is emptied. The data file is written to a temporary file and then swapped in, so a crash during the save never leaves a broken file.

### Taking backups

`maharit backup --source <data file>` reads only the data file. On a running server, changes since the last checkpoint exist only in the WAL. For a complete backup, stop the server with SIGTERM (the data file is saved and the WAL emptied) before backing up.

```bash
systemctl stop maharit          # or kill -TERM <pid>
maharit backup --source /var/lib/maharit/maharit.db --output /backup/maharit_$(date +%Y%m%d).db --compress
systemctl start maharit
```

!!! note "Limitations"
    - Followers (`--replication-role follower`) do not write changes received from the leader to a WAL, because each connection replaces their graph with the leader's snapshot. On promotion, a follower saves its current graph to the data file and then starts using a WAL.
    - Changes written inside a transaction (`begin`) go to the WAL when they run. If the server crashes before a commit or rollback, those changes remain after restart (rolled-back ones do not).
    - `CREATE INDEX` / `CREATE CONSTRAINT` and similar statements run inside a transaction are not undone by a rollback.

## Command-line options

Configure the MaharitDB server with these command-line options. `maharit server --help` shows the list too.

```bash
maharit server [OPTIONS]
```

> **During development**: to run without building first, use `cargo run -p maharit-server -- server [OPTIONS]`.

### Available options

| Option | Short | Default | Description |
|-----------|--------|-----------|------|
| `--host` | `-h` | `127.0.0.1` | Host address to bind to |
| `--port` | `-p` | `7687` | Port to listen on |
| `--max-connections` | `-c` | `100` | Maximum concurrent connections |
| `--data` | `-d` | `maharit.db` | Data file path (env: `MAHARIT_DATA`). The WAL is `<data>.wal` in the same place |
| `--wal` | | `fsync` | WAL mode: `fsync` / `os` / `off` (env: `MAHARIT_WAL`) |
| `--checkpoint-mb` | | `64` | Checkpoint when the WAL exceeds this size (MB). `0` checkpoints only at shutdown |
| `--require-auth` | | off | Require a valid session token for every request ([authentication](auth.md)) |
| `--admin-password` | | none | Initial admin password (env: `MAHARIT_ADMIN_PASSWORD`). Required with `--require-auth` |
| `--tls-cert` / `--tls-key` | | none | Certificate and private key (PEM) for TLS on client connections (and, on a leader, replication connections). Env: `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` ([TLS/SSL](tls.md)) |
| `--tls-min-version` | | `1.2` | Minimum TLS version accepted: `1.2` / `1.3` (env: `MAHARIT_TLS_MIN_VERSION`) |
| `--metrics-bind` | | `127.0.0.1:9090` | Address of the metrics and health-check HTTP server ([metrics](metrics.md)) |
| `--replication-role` | | none | `leader` / `follower` ([replication](../advanced/replication.md)) |
| `--replication-bind` | | `127.0.0.1:7688` | Replication listen address (a leader, or a follower after promotion) |
| `--replication-secret` | | none | Shared secret for replication connections (env: `MAHARIT_REPLICATION_SECRET`) |
| `--leader-addr` | | none | The leader's replication address (required on followers) |
| `--leader-ca` | | none | CA certificate to verify the leader with; connects to the leader over TLS (followers, env: `MAHARIT_LEADER_CA`) |
| `--node-id` | | `node-1` | Node ID, unique within the cluster (env: `MAHARIT_NODE_ID`) |
| `--shard` / `--shard-id` | | none | Start as a shard node ([sharding](../advanced/sharding.md)) |
| `--coordinator` / `--coordinator-port` / `--shards` / `--strategy` | | none | Start as a coordinator |
| `--shard-ca` | | none | CA certificate to verify shards with; connects to shards over TLS (coordinator, env: `MAHARIT_SHARD_CA`) |
| `--shard-user` / `--shard-password` | | `admin` / none | Credentials for logging in to shards with `--require-auth` (coordinator, env: `MAHARIT_SHARD_USER` / `MAHARIT_SHARD_PASSWORD`) |

## Examples

### Basic startup

```bash
# Defaults (127.0.0.1:7687)
maharit server

# Bind to all interfaces
maharit server --host 0.0.0.0 --port 7687

# Set the maximum number of connections
maharit server --host 0.0.0.0 --max-connections 500
```

### Choosing the data file

```bash
# Use /var/lib/maharit/maharit.db and /var/lib/maharit/maharit.db.wal
maharit server --data /var/lib/maharit/maharit.db
```

### WAL settings

```bash
# Skip fsync during a bulk load (still survives a killed process)
maharit server --wal os

# Checkpoint every 256MB
maharit server --checkpoint-mb 256
```

### Log level

Set the log level with the `RUST_LOG` environment variable.

```bash
# Enable debug logs
RUST_LOG=debug maharit server

# Warnings and above only
RUST_LOG=warn maharit server
```

### Metrics address

```bash
maharit server --metrics-bind 0.0.0.0:9090
```

After startup, Prometheus metrics are available at `http://localhost:9090/metrics`.

### Listening with TLS

```bash
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key
```

Enable TLS when the server is used over a network. Listening on a non-loopback address without TLS logs a warning at startup. The metrics HTTP server is not covered by TLS ([TLS/SSL](tls.md)).

## Environment variables

Environment variables can be used instead of command-line options. If both are given, the option wins.

| Environment variable | Option |
|---------|----------------|
| `MAHARIT_DATA` | `--data` |
| `MAHARIT_WAL` | `--wal` |
| `MAHARIT_ADMIN_PASSWORD` | `--admin-password` |
| `MAHARIT_REPLICATION_SECRET` | `--replication-secret` |
| `MAHARIT_NODE_ID` | `--node-id` |
| `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` | `--tls-cert` / `--tls-key` |
| `MAHARIT_TLS_MIN_VERSION` | `--tls-min-version` |
| `MAHARIT_LEADER_CA` | `--leader-ca` |
| `RUST_LOG` | Log level |

```bash
export MAHARIT_DATA=/var/lib/maharit/maharit.db
export MAHARIT_WAL=fsync
maharit server --host 0.0.0.0
```

## Log output

The server writes structured logs in JSON. Some messages, such as the startup message, are also written as plain text.

```json
{"timestamp":"2026-11-08T05:14:29.548490Z","level":"INFO","fields":{"message":"Server listening","address":"0.0.0.0:7687"},"target":"maharit::tcp_server"}
```

To redirect logs to a file:

```bash
maharit server > /var/log/maharit/server.log 2>&1
```

With `systemd`, collect them with `journald`.

## Example systemd service

```ini
[Unit]
Description=MaharitDB Graph Database Server
After=network.target

[Service]
Type=simple
User=maharit
Group=maharit
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --data /var/lib/maharit/maharit.db
# Leave time to save the data file at shutdown
KillSignal=SIGTERM
TimeoutStopSec=60s
Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
```
