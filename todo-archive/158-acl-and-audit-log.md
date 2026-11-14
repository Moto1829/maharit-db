# タスク 158: ユーザーの永続化・監査ログ・ラベル単位のアクセス制御（ACL）をサーバーに組み込む

## 現状（2026-11-12、task148 で確認）
- ユーザーはメモリ上にだけあり、再起動で消える（`AuthManager::save_to_file` / `load_from_file` はあるが未使用）
- `audit.rs` に `AuditLogger`（ファイル出力・ローテーション）があるが、どこからも使われていない
- `auth.rs` に `AclManager` / `AclRule`（ラベル・プロパティ単位の Allow / Deny）があるが、クエリの実行とつながっておらず、
  規則を設定する文（`GRANT` / `REVOKE` / `DENY` / `SHOW ACL`）も無い。docs/operations/acl.md は「未実装」と書き直した

## 作業（案）
- [x] ユーザーの永続化: `--users-file <PATH>` で `AuthManager` を保存・読み込み
- [x] 監査ログ: `--audit-log <PATH>` でログイン成功・失敗、ユーザー管理、権限エラーを記録
- [→] ACL: 規則の設定文とその保存先を設計し、MATCH の結果やプロパティ参照に適用する（範囲が大きいので設計を先に）

## 結果（2026-11-14）
- `--users-file <PATH>`（`MAHARIT_USERS_FILE`、`--require-auth` が必要）: 無ければ `--admin-password` の admin でファイルを作成、
  あれば読み込んで `--admin-password` は無視（その旨を表示）。有効な admin がいない・旧版（v1）・壊れたファイルなら起動しない。
  変更のたびに一時ファイル経由で保存し、Unix では 0600。保存の失敗はログに出す（以前は黙って無視）
- `--audit-log <PATH>`（`MAHARIT_AUDIT_LOG`）: login_success / login_failure、user_created / user_modified / user_deleted（`by` 付き）、
  permission_denied（`action`: write / manage_users / promote / begin_write_transaction）を JSON Lines で記録。コーディネーターは対象外
- E2E: `scripts/users_audit_test.py`（自分でサーバーを起動。`run_e2e_local.sh` に追加）
- docs: operations/auth.md・server-config.md（日英）
- ACL は設計が必要なので server/173 に移した（設計案を記載）
