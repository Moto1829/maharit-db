# タスク 148: 運用ドキュメント（docs/operations/）を実装と照合する

## 背景
2026-11-08、`server-config.md` を書き直した際に、オプション表の大半が実在しないフラグだった（`--log-level`、`--tls-cert`、`--metrics-port`、`--enable-replication`）。同じ傾向が他のページにもある。

## 確認済みの不一致
- `backup.md`: `--data-dir`、`--compress gzip`、`--type incremental`、`CALL db.backup(...)`、`maharit wal list`、`restore --wal-dir --target-time / --target-lsn` は存在しない。実際は `maharit backup --source <DB> --output <PATH> [--compress|--compress-zstd] [--description]`、`maharit backup --list <PATH>`、`maharit restore <BACKUP> --output <DB>`。増分バックアップ・PITR は `maharit-storage` の Rust API のみ
- `backup.md`「バックアップの内容」: storage/149 で定義がデータファイルに入るようになり、記述を直した（2026-11-09）
- `tls.md`: server/147 で実装に合わせて書き直し済み（2026-11-08）

## 作業
- [x] operations/ 配下の各ページ（auth / acl / metrics / tracing / backup / docker）のコマンド・フラグ・出力例を `maharit <cmd> --help` と実機で照合して直す（2026-11-12）
  - backup: 実在するオプションに書き直し。稼働中のバックアップは最後のチェックポイントまで（WAL は含まない）、リストア後は古い WAL を消す、を明記
  - auth: 構文を `CREATE USER x SET PASSWORD '…' SET ROLE r` に、フラグを `--require-auth` / `--admin-password` に、セッションは無操作 30 分、ハッシュは PBKDF2、ユーザーは再起動で消える、LOGOUT・監査ログは無い
  - acl: 未実装と明記（server/158）
  - metrics: 実際のメトリクス名に。値が 0 のままの不具合は bug/156
  - tracing: OpenTelemetry 送信は無いので「ログ」のページにした（JSON 構造化ログ・RUST_LOG）
  - docker: Dockerfile の既定コマンドの不具合を bug/157 で修正し、それに合わせて書き直し
  - README / index / architecture の ACL・OpenTelemetry・増分バックアップの記述も修正
- [ ] 英語版（task 140）はこのタスクの後に訳す
