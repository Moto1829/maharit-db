# タスク 148: 運用ドキュメント（docs/operations/）を実装と照合する

## 背景
2026-11-08、`server-config.md` を書き直した際に、オプション表の大半が実在しないフラグだった（`--log-level`、`--tls-cert`、`--metrics-port`、`--enable-replication`）。同じ傾向が他のページにもある。

## 確認済みの不一致
- `backup.md`: `--data-dir`、`--compress gzip`、`--type incremental`、`CALL db.backup(...)`、`maharit wal list`、`restore --wal-dir --target-time / --target-lsn` は存在しない。実際は `maharit backup --source <DB> --output <PATH> [--compress|--compress-zstd] [--description]`、`maharit backup --list <PATH>`、`maharit restore <BACKUP> --output <DB>`。増分バックアップ・PITR は `maharit-storage` の Rust API のみ
- `backup.md`「バックアップの内容」: インデックス・スキーマ制約は含まれない（`save_concurrent` は `indexes: vec![]`）
- `tls.md`: TLS はサーバーに未組み込み（server/147）

## 作業
- [ ] operations/ 配下の各ページ（auth / acl / metrics / tracing / backup / docker / tls）のコマンド・フラグ・出力例を `maharit <cmd> --help` と実機で照合して直す
- [ ] 英語版（task 140）はこのタスクの後に訳す
