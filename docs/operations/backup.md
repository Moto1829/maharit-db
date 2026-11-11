---
title: バックアップ・リストア
parent: サーバー・運用
nav_order: 7
---

# バックアップ・リストア

`maharit backup` はデータファイル（`--data` で指定したファイル）から 1 つのバックアップファイルを作り、`maharit restore` はバックアップファイルからデータファイルを作ります。圧縮は gzip と zstd に対応しています。

## データファイルと WAL

サーバーは変更をまず WAL（`<data>.wal`）に書き、次のときにデータファイルへまとめて保存します（チェックポイント）。

- WAL が `--checkpoint-mb`（既定 64 MB）を超えたとき
- サーバーを停止したとき（SIGTERM / Ctrl+C）

`maharit backup` が読むのは**データファイルだけ**です。稼働中のサーバーのデータファイルからバックアップすると、**最後のチェックポイントの時点**の内容になり、それ以降の変更（WAL にだけある変更）は含まれません。まだ一度もチェックポイントしていなければ、データファイルが無いためエラーになります。

すべての変更を含めるには、サーバーを停止してからバックアップしてください。データファイルは一時ファイルに書いてから置き換えるので、稼働中に読んでも書きかけの内容を読むことはありません。

## バックアップを作る

```bash
# サーバーを停止してからバックアップ
systemctl stop maharit
maharit backup --source /var/lib/maharit/maharit.db --output /backup/maharit_20260101.bak
systemctl start maharit

# gzip 圧縮・説明付き
maharit backup --source /var/lib/maharit/maharit.db \
  --output /backup/maharit_20260101.bak.gz --compress --description "nightly"

# zstd 圧縮
maharit backup --source /var/lib/maharit/maharit.db \
  --output /backup/maharit_20260101.bak.zst --compress-zstd
```

| オプション | 説明 |
|---|---|
| `-s`, `--source <DB_PATH>` | バックアップするデータファイル（必須） |
| `-o`, `--output <PATH>` | 出力するバックアップファイル（既定: `maharit_backup.db`） |
| `-z`, `--compress` | gzip で圧縮する |
| `--compress-zstd` | zstd で圧縮する |
| `-d`, `--description <TEXT>` | バックアップに説明を付ける |

出力例:

```text
Backup created successfully:
  Output:      /backup/maharit_20260101.bak.gz
  Nodes:       100000
  Edges:       500000
  Compression: gzip
```

### バックアップの内容

- 全ノード・リレーションシップとそのラベル・プロパティ
- インデックス・制約・全文検索インデックスの定義（インデックスの中身はリストア後の起動時に作り直す）
- 作成日時・ノード数・リレーションシップ数・圧縮形式・説明

ユーザーとパスワード（`--admin-password` / `CREATE USER`）はデータファイルに保存されないため、バックアップにも含まれません。

## バックアップを確認する

```bash
# メタデータを表示
maharit backup --list /backup/maharit_20260101.bak.gz

# 全体を読み込んで壊れていないか検証（データファイルは作らない）
maharit restore --verify /backup/maharit_20260101.bak.gz
```

出力例:

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

## リストア

```bash
systemctl stop maharit
maharit restore /backup/maharit_20260101.bak.gz --output /var/lib/maharit/maharit.db
rm -f /var/lib/maharit/maharit.db.wal
systemctl start maharit
```

| オプション | 説明 |
|---|---|
| `<BACKUP_PATH>` / `-i`, `--input <BACKUP_PATH>` | リストアするバックアップファイル。圧縮形式は自動で判別する |
| `-o`, `--output <DB_PATH>` | 書き出すデータファイル。省略すると、読み込めることを確かめて件数を表示するだけでファイルは作らない |
| `--verify` | 検証だけを行う |

!!! warning "リストアの注意"
    - サーバーを停止してから行ってください。稼働中のサーバーは停止時にデータファイルを自分の内容で上書きします
    - 古い WAL（`<data>.wal`）は削除してください。残っていると、起動時にリストアしたデータの上へ古い変更が再適用されます
    - `--output` に既存のファイルを指定すると上書きします

## 定期バックアップ

稼働中のバックアップは最後のチェックポイントまでの内容になるため、変更を確実に含めたい場合は停止を伴う手順にします。停止できない場合は、`--checkpoint-mb` を小さくするとチェックポイントの間隔が短くなり、バックアップに含まれない変更が減ります。

```bash
# /etc/cron.d/maharit-backup — 毎日 2 時（停止を伴う）
0 2 * * * root systemctl stop maharit && \
  /usr/local/bin/maharit backup --source /var/lib/maharit/maharit.db \
    --output /backup/maharit_$(date +\%Y\%m\%d).bak.gz --compress; \
  systemctl start maharit
```

古いバックアップを削除する例:

```bash
find /backup -name "maharit_*.bak.gz" -mtime +30 -delete
```

## 増分バックアップ・ポイントインタイムリカバリ

`maharit-storage` クレートの Rust API には、増分バックアップ（`Backup::create_incremental` / `Backup::restore_incremental`）と、指定時刻までの復元（`Backup::restore_to_point_in_time`）があります。`maharit` コマンドからは使えません。

## 運用の注意

- バックアップファイルは別のストレージ（別サーバー、クラウドストレージなど）にもコピーしてください
- 定期的にリストアを試し、`--verify` だけでなく実際に起動してデータを確認してください
