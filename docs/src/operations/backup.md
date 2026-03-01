# バックアップ・リストア

MaharitDB は gzip 圧縮付きのフルバックアップ、増分バックアップ、および WAL を使ったポイントインタイムリカバリ（PITR）をサポートしています。

## フルバックアップ

### バックアップの実行

```bash
# フルバックアップを実行
maharit backup --output /backup/maharit_20240101.bak.gz

# 特定のデータディレクトリからバックアップ
maharit backup \
  --data-dir /var/lib/maharit \
  --output /backup/maharit_20240101.bak.gz \
  --compress gzip
```

Cypher コマンドからバックアップを実行することもできます：

```cypher
CALL db.backup("/backup/maharit_20240101.bak.gz")
```

### バックアップの内容

バックアップファイルには以下が含まれます：

- グラフの全ノードとエッジ（バイナリシリアライズ形式）
- インデックスデータ
- スキーマ制約
- バックアップ時刻のタイムスタンプ

## リストア

```bash
# バックアップからリストア
maharit restore --input /backup/maharit_20240101.bak.gz

# 別のデータディレクトリにリストア
maharit restore \
  --input /backup/maharit_20240101.bak.gz \
  --data-dir /var/lib/maharit_restored
```

注意: リストアを実行する前にサーバーを停止してください。リストアは既存のデータを上書きします。

## 増分バックアップ

フルバックアップの後、変更されたデータのみをバックアップします。

```bash
# フルバックアップ（ベースライン）
maharit backup --output /backup/full_20240101.bak.gz --type full

# 増分バックアップ（前回バックアップ以降の変更）
maharit backup \
  --output /backup/incr_20240102.bak.gz \
  --type incremental \
  --since /backup/full_20240101.bak.gz
```

### 増分バックアップのリストア

```bash
# フルバックアップをリストア
maharit restore --input /backup/full_20240101.bak.gz

# 増分バックアップを適用
maharit restore \
  --input /backup/incr_20240102.bak.gz \
  --apply-incremental
```

## ポイントインタイムリカバリ（PITR）

WAL ログを使用して、特定の時点の状態に復元します。

### WAL ログの確認

```bash
maharit wal list --data-dir /var/lib/maharit
```

出力例：

```
WAL segments:
  000000001 2024-01-01 00:00:00 - 2024-01-01 12:00:00 (123 MB)
  000000002 2024-01-01 12:00:00 - 2024-01-02 00:00:00 (98 MB)
  000000003 2024-01-02 00:00:00 - 現在 (45 MB)
```

### 特定時点へのリカバリ

```bash
# 2024-01-01 18:00:00 時点にリカバリ
maharit restore \
  --input /backup/full_20240101.bak.gz \
  --wal-dir /var/lib/maharit/wal \
  --target-time "2024-01-01T18:00:00Z"

# 特定の WAL シーケンス番号までリカバリ
maharit restore \
  --input /backup/full_20240101.bak.gz \
  --wal-dir /var/lib/maharit/wal \
  --target-lsn 1234567
```

## バックアップスケジュール

`cron` を使用して定期的なバックアップを設定できます。

```bash
# /etc/cron.d/maharit-backup
# 毎日午前 2 時にフルバックアップ
0 2 * * * maharit /usr/local/bin/maharit backup \
  --data-dir /var/lib/maharit \
  --output /backup/maharit_$(date +%Y%m%d).bak.gz

# 毎時間増分バックアップ
0 * * * * maharit /usr/local/bin/maharit backup \
  --data-dir /var/lib/maharit \
  --output /backup/incr_$(date +%Y%m%d_%H).bak.gz \
  --type incremental
```

## バックアップの検証

バックアップファイルが正常に作成されたか確認します。

```bash
maharit backup verify --input /backup/maharit_20240101.bak.gz
```

出力例：

```
Backup file: /backup/maharit_20240101.bak.gz
Format: gzip-compressed binary
Timestamp: 2024-01-01T02:00:00Z
Nodes: 100,000
Edges: 500,000
Indexes: 5
Constraints: 10
Checksum: OK
```

## バックアップのローテーション

古いバックアップを自動削除するスクリプト例：

```bash
#!/bin/bash
BACKUP_DIR=/backup
KEEP_DAYS=30

# 30 日以上前のバックアップを削除
find $BACKUP_DIR -name "maharit_*.bak.gz" -mtime +$KEEP_DAYS -delete
find $BACKUP_DIR -name "incr_*.bak.gz" -mtime +7 -delete  # 増分は 7 日保持
```

## 重要な注意事項

- バックアップ中もサーバーは稼働し続けますが、バックアップ期間中は書き込み操作のレイテンシが若干増加することがあります
- バックアップファイルは必ず別のストレージ（別サーバー、クラウドストレージなど）にコピーしてください
- 定期的にリストアテストを実施してバックアップの有効性を確認してください
