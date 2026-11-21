# maharit-server

バイナリ `maharit`（REPL / `server` / `backup` / `restore` / `admin`）と TCP サーバー。

## ファイル構成

- `main.rs`: CLI。`repl.rs`: REPL（履歴 `.maharit_history` は git 管理外）
- `tcp_server.rs` + `tcp_server/`: `TcpServer` と起動処理。`protocol.rs`（`Request` / `Response`）/ `connection.rs`（1 接続の処理とリクエストの振り分け）/ `query.rs`（クエリ実行・WAL・レプリケーション送出）/ `session.rs`（セッション・権限・ユーザー管理と ACL の文）/ `state.rs`（永続化・共有マネージャ）
- `durable_wal.rs`: ディスク WAL（`<data>.wal`）。書き込みゲート内で追記し、ゲート外で fsync（グループコミット）してから応答する。起動時はスナップショット → WAL 再生。WAL ヘッダーのスナップショット SHA-256 が合わない WAL は捨てる
- `mutation_log.rs`: `RecordingGraph`。書き込みの変更ログ（`WalEntryData`）と取り消し用の記録を作る。WAL とレプリケーションは同じログを使う
- `replication.rs`: リーダー / フォロワー。`schema.rs`: 索引・制約の定義（WAL では `SetSchema` で全体を置き換え）
- `auth.rs` / `audit.rs` / `tls.rs` / `http_server.rs`（`/metrics`・`/health`）/ `coordinator.rs`（シャーディング）

## 知らないと間違えること

- プロトコルは 4 バイト長（ビッグエンディアン）+ JSON。`Request` / `Response` のフィールドの削除・改名・型の変更は互換を壊す（`feat!:`）。省略可能なフィールドの追加は `feat:`（`#[serde(default)]` を付ける）
- 新しいリクエスト型やクエリの経路を足すときは、`check_session_user`（認証）→ `authorize_query`（ロール）→ `session_policy`（ACL）→ `audit_log` / `audit_denied`（監査）を通す。`connection.rs` の `Request::Query` の分岐が手本
- 文の実行は `run_statement`（`spawn_blocking`）で tokio のワーカーの外で行う。書き込みは `write_gate` を取ったまま実行する
- WAL は ID で対象を指すので、スナップショットの読み込み（`load_concurrent`）はノード・エッジ ID を保存時のまま復元する。振り直さない
- ACL の規則はサーバーごと。フォロワーには送られない
- Linux 専用コード（`cfg(target_os = "linux")`、`metrics.rs` など）は macOS の clippy で検査されない。CI（ubuntu）で初めて落ちることがある
- TLS は rustls 0.23。`builder_with_protocol_versions` を使う（`versions` フィールドは private）
- テストで環境変数を変える（`set_var`）ときは `// SAFETY:` に前提を書く（workspace の lint）

## テスト

- 単体・結合テストは `tcp_server/tests/`（`integration.rs` は実際に TCP で接続する）
- `replication_diff_test.rs`: リーダーとフォロワーの状態が一致するかを乱数で確かめる（`MAHARIT_DIFF_SEEDS`）。`concurrency_stress_test.rs`: 並行書き込み（`MAHARIT_STRESS_SEED`）
- E2E は `bash scripts/run_e2e_local.sh`（要 `cargo build --release -p maharit-server`）
