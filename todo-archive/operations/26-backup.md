# Task 26: バックアップ・リストア

## 概要
データベースのバックアップとリストア機能を実装する。

## 実装内容

### オンラインバックアップ
- [x] サーバー稼働中のバックアップ
- [x] スナップショットの作成
- [x] 増分バックアップ（IncrementalBackupMetadata / create_incremental / restore_incremental）

### バックアップ形式
- [x] フルバックアップ（データファイル + WAL）
- [x] 圧縮オプション（gzip）
- [x] 圧縮オプション（zstd）
- [x] バックアップのメタデータ（タイムスタンプ、サイズ）

### リストア
- [x] バックアップからの復元
- [x] ポイントインタイムリカバリ（WAL適用）（restore_to_point_in_time）
- [x] 復元の検証

### CLIコマンド
- [x] `maharit backup --output backup.tar.gz`
- [x] `maharit restore --input backup.tar.gz`
- [x] `maharit backup --list` - バックアップ一覧

### スケジュールバックアップ
- [x] 定期バックアップの設定
- [x] 古いバックアップの自動削除
- [x] バックアップ完了通知

## API
```rust
// プログラムからのバックアップ
let metadata = Backup::create(&graph, "backup.db", &BackupOptions::default())?;

// 圧縮バックアップ
let metadata = Backup::create(&graph, "backup.db.gz", &BackupOptions::compressed())?;

// メタデータ確認
let meta = Backup::metadata("backup.db")?;

// リストア
let graph = Backup::restore("backup.db")?;

// バックアップ検証
let valid = Backup::verify("backup.db")?;
```

## 依存
- `09-persistence-format.md` が完了していること
- `10-wal.md` が完了していること

## 対象クレート
`maharit-storage`, `maharit-server`
