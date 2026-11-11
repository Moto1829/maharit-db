# タスク 158: ユーザーの永続化・監査ログ・ラベル単位のアクセス制御（ACL）をサーバーに組み込む

## 現状（2026-11-12、task148 で確認）
- ユーザーはメモリ上にだけあり、再起動で消える（`AuthManager::save_to_file` / `load_from_file` はあるが未使用）
- `audit.rs` に `AuditLogger`（ファイル出力・ローテーション）があるが、どこからも使われていない
- `auth.rs` に `AclManager` / `AclRule`（ラベル・プロパティ単位の Allow / Deny）があるが、クエリの実行とつながっておらず、
  規則を設定する文（`GRANT` / `REVOKE` / `DENY` / `SHOW ACL`）も無い。docs/operations/acl.md は「未実装」と書き直した

## 作業（案）
- [ ] ユーザーの永続化: `--users-file <PATH>` で `AuthManager` を保存・読み込み
- [ ] 監査ログ: `--audit-log <PATH>` でログイン成功・失敗、ユーザー管理、権限エラーを記録
- [ ] ACL: 規則の設定文とその保存先を設計し、MATCH の結果やプロパティ参照に適用する（範囲が大きいので設計を先に）
