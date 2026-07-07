# bug/71: 認証の有効化フラグ・RBAC 強制・初期 admin パスワード設定

## 概要
以下の複合的な認可の欠陥に対応する。

1. **認証を有効化する手段が存在しない (CRITICAL)**
   `ServerConfig::default()` は `require_auth: false` で、`main.rs` に有効化フラグが
   なかった。配布バイナリ `maharit server` は常に無認証で全リクエストを受理していた。

2. **RBAC が一切強制されていない (HIGH)**
   `check_permission` は実装済みだがサーバーからどこからも呼ばれておらず、
   `check_session` はトークンの有効性のみ検証していた。結果、`ReadOnly` ロールでも
   書き込みクエリを実行できた（権限昇格）。

3. **デフォルト認証情報 admin/admin (MEDIUM)**
   `AuthManager::new()` が常に admin/admin を作成し、変更手段がなかった。

## 対応
- `main.rs` に `--require-auth` フラグを追加（`ServerConfig::require_auth` を有効化）。
- `--admin-password <PW>` / `MAHARIT_ADMIN_PASSWORD` で初期 admin パスワードを設定。
  `--require-auth` 有効時にパスワード未指定なら起動を拒否（admin/admin をネットワーク公開しない）。
- `AuthManager::with_admin_password()` と `AuthManager::check_role_permission()`（インスタンス不要）を追加。
- `TcpServer::with_auth()` で設定済み AuthManager を注入可能に。
- `check_session` を `Result<Role, Response>` に変更しロールを返す。
- `authorize_query()` を追加し、書き込みクエリを `ReadOnly` ロールから拒否。
  Query / StreamQuery / 書き込み BeginTransaction に RBAC を配線。
- ヘルプテキスト更新、ユニットテスト 6 件追加。

## ステータス
完了（server テスト 223 件パス）
