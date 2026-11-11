---
title: Backup and restore
parent: Server and operations
nav_order: 7
source_commit: ed074894
---

# Backup and restore

`maharit backup` creates a single backup file from a data file (the file given with `--data`), and `maharit restore` creates a data file from a backup file. gzip and zstd compression are supported.

## The data file and the WAL

The server first writes changes to the WAL (`<data>.wal`) and saves them to the data file in bulk (a checkpoint) when:

- the WAL exceeds `--checkpoint-mb` (64 MB by default)
- the server stops (SIGTERM / Ctrl+C)

`maharit backup` reads **only the data file**. Backing up the data file of a running server gives the state as of **the last checkpoint**; later changes (those only in the WAL) are not included. If there has been no checkpoint yet, there is no data file and the backup fails.

To include every change, stop the server before backing up. The data file is written to a temporary file and then swapped in, so reading it while the server runs never sees a half-written file.

## Creating a backup

```bash
# Stop the server, then back up
systemctl stop maharit
maharit backup --source /var/lib/maharit/maharit.db --output /backup/maharit_20260101.bak
systemctl start maharit

# gzip compression with a description
maharit backup --source /var/lib/maharit/maharit.db \
  --output /backup/maharit_20260101.bak.gz --compress --description "nightly"

# zstd compression
maharit backup --source /var/lib/maharit/maharit.db \
  --output /backup/maharit_20260101.bak.zst --compress-zstd
```

| Option | Description |
|---|---|
| `-s`, `--source <DB_PATH>` | Data file to back up (required) |
| `-o`, `--output <PATH>` | Backup file to write (default `maharit_backup.db`) |
| `-z`, `--compress` | Compress with gzip |
| `--compress-zstd` | Compress with zstd |
| `-d`, `--description <TEXT>` | Add a description to the backup |

Example output:

```text
Backup created successfully:
  Output:      /backup/maharit_20260101.bak.gz
  Nodes:       100000
  Edges:       500000
  Compression: gzip
```

### What a backup contains

- Every node and relationship with its labels and properties
- Definitions of indexes, constraints, and full-text indexes (index contents are rebuilt at startup after a restore)
- Creation time, node count, relationship count, compression, and description

Users and passwords (`--admin-password` / `CREATE USER`) are not saved in the data file, so they are not in backups either.

## Checking a backup

```bash
# Show the metadata
maharit backup --list /backup/maharit_20260101.bak.gz

# Read the whole file to verify it (does not create a data file)
maharit restore --verify /backup/maharit_20260101.bak.gz
```

Example output:

```text
Backup: /backup/maharit_20260101.bak.gz
  Version:     2
  Created:     2026-01-01 02:00:00 UTC
  Nodes:       100000
  Edges:       500000
  Compression: gzip
  Description: nightly
```

```text
Backup verification passed: /backup/maharit_20260101.bak.gz
```

## Restoring

```bash
systemctl stop maharit
maharit restore /backup/maharit_20260101.bak.gz --output /var/lib/maharit/maharit.db
rm -f /var/lib/maharit/maharit.db.wal
systemctl start maharit
```

| Option | Description |
|---|---|
| `<BACKUP_PATH>` / `-i`, `--input <BACKUP_PATH>` | Backup file to restore. The compression is detected automatically |
| `-o`, `--output <DB_PATH>` | Data file to write. Without it, the backup is only loaded and its counts printed; no file is created |
| `--verify` | Only verify |

!!! warning "Before restoring"
    - Stop the server first. A running server overwrites the data file with its own contents when it stops
    - Delete the old WAL (`<data>.wal`). If it remains, its old changes are replayed on top of the restored data at startup
    - An existing file given as `--output` is overwritten

## Scheduled backups

Backups of a running server only contain changes up to the last checkpoint, so stop the server when you need every change. If you cannot stop it, a smaller `--checkpoint-mb` shortens the time between checkpoints and reduces the changes missing from a backup.

```bash
# /etc/cron.d/maharit-backup — every day at 2:00 (with a stop)
0 2 * * * root systemctl stop maharit && \
  /usr/local/bin/maharit backup --source /var/lib/maharit/maharit.db \
    --output /backup/maharit_$(date +\%Y\%m\%d).bak.gz --compress; \
  systemctl start maharit
```

Deleting old backups:

```bash
find /backup -name "maharit_*.bak.gz" -mtime +30 -delete
```

## Incremental backups and point-in-time recovery

The Rust API of the `maharit-storage` crate has incremental backups (`Backup::create_incremental` / `Backup::restore_incremental`) and restoring up to a given time (`Backup::restore_to_point_in_time`). They are not available from the `maharit` command.

## Operational notes

- Copy backup files to separate storage as well (another server, cloud storage, and so on)
- Practice restores regularly: start a server on the restored data and check it, not just `--verify`
