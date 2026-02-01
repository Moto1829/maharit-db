# バックアップ・リストア

## 概要
データベースのバックアップとリストア機能を実装する。

## 実装内容

### オンラインバックアップ
- [ ] サーバー稼働中のバックアップ
- [ ] スナップショットの作成
- [ ] 増分バックアップ（将来的）

### バックアップ形式
- [ ] フルバックアップ（データファイル + WAL）
- [ ] 圧縮オプション（gzip/zstd）
- [ ] バックアップのメタデータ（タイムスタンプ、サイズ）

### リストア
- [ ] バックアップからの復元
- [ ] ポイントインタイムリカバリ（WAL適用）
- [ ] 復元の検証

### CLIコマンド
- [ ] `maharit backup --output backup.tar.gz`
- [ ] `maharit restore --input backup.tar.gz`
- [ ] `maharit backup --list` - バックアップ一覧

### スケジュールバックアップ
- [ ] 定期バックアップの設定
- [ ] 古いバックアップの自動削除
- [ ] バックアップ完了通知

## API
```rust
// プログラムからのバックアップ
let backup = Backup::create(&db, BackupOptions::default())?;
backup.save("backup.tar.gz")?;

// リストア
Backup::restore("backup.tar.gz", &restore_options)?;
```

## 依存
- `09-persistence-format.md` が完了していること
- `10-wal.md` が完了していること

## 対象クレート
`maharit-storage`, `maharit-server`
