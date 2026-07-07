# bug/76: シャードコーディネーターの認証・RBAC・メッセージサイズ上限

## 概要
`--coordinator` モードで起動する `ShardCoordinatorServer` はクラスタの正面玄関だが、
認証が皆無で、任意のクライアントがクエリを送りシャードにファンアウトできた（認証バイパス）。
また `read_request` の `vec![0u8; msg_len]` に上限が無く、メモリ枯渇 DoS の余地があった。
`main.rs` の coordinator 分岐は認証解決前に early return しており、bug/71 の認証も未適用だった。

## 対応
- `CoordinatorConfig.require_auth` を追加。
- `ShardCoordinatorServer` に `AuthManager` を保持し `with_auth()` を追加。
- `CoordRequest::Login` / `Query.session_token`、`CoordResponse::LoggedIn` / `AuthError` を追加。
- `handle_connection` に Login 処理と `check_query_auth`（セッション検証 + RBAC）を配線。
  ReadOnly ロールの書き込みクエリを拒否。
- `read_request` に `MAX_MESSAGE_SIZE`(64MiB) 上限を追加。
- 認証解決を `resolve_admin_auth()` ヘルパーに切り出し、coordinator 分岐とメインパスで共有。
  coordinator でも `--require-auth` / `--admin-password` が有効化され、未指定なら起動拒否。
- テスト 6 件追加（role_label + check_query_auth 各種）。

## 影響
- 重大度: HIGH（認証バイパス）+ MEDIUM（DoS）
- `require_auth` 未設定時は従来どおり無認証（互換維持）。

## ステータス
完了（server テスト 237 件パス）
