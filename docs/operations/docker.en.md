---
title: Docker
parent: Server and operations
nav_order: 8
source_commit: ed074894
---

# Using Docker

The repository's `Dockerfile` builds an image that contains `maharit`.

## Building the image

```bash
docker build -t maharit-db .
```

What the image contains:

- `/app/maharit`: the binary
- `/data`: the data volume (the data file is `/data/maharit.db`, the WAL `/data/maharit.db.wal`)
- Default command: `maharit server --host 0.0.0.0 --metrics-bind 0.0.0.0:9090`
- Ports: 7687 (clients), 9090 ([metrics and health checks](metrics.md))
- `HEALTHCHECK`: checks `http://localhost:9090/health` every 30 seconds
- Runs as the `maharit` user (not root)

## Starting the server

```bash
docker run -d --name maharit \
  -p 7687:7687 -p 127.0.0.1:9090:9090 \
  -v maharit-data:/data \
  -e MAHARIT_ADMIN_PASSWORD="$MAHARIT_ADMIN_PASSWORD" \
  maharit-db server --host 0.0.0.0 --metrics-bind 0.0.0.0:9090 --require-auth
```

- Without `-v maharit-data:/data`, deleting the container deletes the data too
- A command you give replaces the default command, so include `--host 0.0.0.0` and `--metrics-bind 0.0.0.0:9090`. Binding to `127.0.0.1` inside a container makes the server unreachable from outside
- For authentication, TLS, and other options, see [Server configuration](server-config.md). Passing passwords as environment variables rather than command-line arguments keeps them out of `docker inspect` and process listings

Stop the server with `docker stop`. On SIGTERM the server saves the data file before exiting. If the default grace period (10 seconds) is not enough for a large graph, extend it, for example `docker stop -t 60 maharit`.

## Using the REPL

```bash
docker run -it --rm maharit-db /app/maharit
```

REPL data lives only in memory and is lost when you exit.

## docker-compose

`docker-compose.yml` defines these services:

| Service | Content |
|---|---|
| `maharit-server` | The server (publishes 7687 / 9090; volume `maharit-data` on `/data`) |
| `maharit` | The REPL (`docker-compose run maharit`) |
| `maharit-viz` | The visualization web app (8080, connects to `maharit-server`) |

```bash
docker-compose up -d maharit-server
docker-compose run --rm maharit
```

A replication setup with one leader and two followers is in `docker-compose.replication.yml` (see [Replication](../advanced/replication.md)).

## Environment variables

The environment variables of `maharit server` work as is (see [Server configuration](server-config.md)).

| Variable | Default in the image | Description |
|---|---|---|
| `MAHARIT_DATA` | `/data/maharit.db` | Data file path |
| `MAHARIT_WAL` | (`fsync`) | WAL mode |
| `MAHARIT_ADMIN_PASSWORD` | none | Admin password for `--require-auth` |
| `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` | none | TLS certificate and private key (paths of files mounted into the container) |
| `RUST_LOG` | (`info`) | [Log](tracing.md) level |

## Backups

Stop the server container, then run `maharit backup` in another container that mounts the same volume (see [Backup and restore](backup.md)).

```bash
docker stop maharit
docker run --rm -v maharit-data:/data -v "$PWD/backup:/backup" maharit-db \
  /app/maharit backup --source /data/maharit.db --output /backup/maharit.bak.gz --compress
docker start maharit
```

Make sure the container's `maharit` user can write to the host directory mounted at `/backup`.

## When you cannot connect

1. Is the container running? `docker ps`
2. Port mapping: `docker port maharit`
3. Logs: `docker logs maharit` (look for `Server listening on 0.0.0.0:7687`)
4. Did you override the command without `--host 0.0.0.0`?
