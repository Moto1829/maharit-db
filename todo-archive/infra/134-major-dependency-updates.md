# タスク 134: メジャーバージョンが上がった依存の更新

## 概要（2026-11-03 の `cargo update --dry-run --verbose`）

互換範囲外で新しい版が出ている直接依存。既知の脆弱性は無い（quick-xml は task 129、lindera は task 132 で扱う）。

| クレート | 現在 | 最新 | 使っている箇所 / 注意 |
|---|---|---|---|
| axum / axum-server | 0.7 / 0.7 | 0.8 / 0.8 | maharit-viz。ルートのパス構文（`/:id` → `/{id}`）が変わる。axum-server 0.7 がメンテ終了の rustls-pemfile を引き込む最後の経路（task 130 で直接依存は削除済み） |
| tower-http / tower-cookies | 0.6 / 0.10 | 0.7 / 0.11 | maharit-viz。axum と同時に上げる |
| rustyline | 15 | 19 | REPL。Windows 向け依存の clipboard-win 5.4.1 が yanked |
| getrandom / pbkdf2 / sha2 | 0.2 / 0.12 / 0.10 | 0.4 / 0.13 / 0.11 | `auth.rs` のパスワードハッシュとトークン生成。まとめて上げ、既存のハッシュが検証できることをテストで確認する |
| webpki-roots | 0.26（1.x への互換 shim） | 1.0 | `tls.rs`。直接 1.x を使う |
| zstd | 0.13 | 0.14 | バックアップ圧縮。旧版で作ったバックアップが復元できることを確認する |
| toml / colored | 0.8 / 2 | 1.x / 3 | 設定ファイルの読み込み / REPL の表示 |

ルートの `[workspace.dependencies]` にある `tokio-tungstenite = "0.24"` は、どのクレートも使っていない（`Cargo.lock` にも無い）ので削除する。

## 作業
- 関連するものをまとめて 1 つずつ上げ、`cargo test --workspace` と E2E を通す
- 認証（getrandom / pbkdf2 / sha2）とバックアップ（zstd）は、既存データとの互換テストを必ず入れる

## 対応内容（2026-11-03）

| 更新 | コード変更 | 確認 |
|---|---|---|
| axum 0.8 / axum-server 0.8 / tower-http 0.7 / tower-cookies 0.11 | なし（パス引数のルートが無いため） | viz を実際に起動し、HTTP（health/info/query/静的ファイル）・`--auth` で未ログイン 401・TLS（テスト用自己署名証明書）を確認。axum-server 0.8 で `rustls-pemfile` が依存から消え、監査警告が 1 件減った |
| rustyline 19 | なし | パイプ入力と疑似 TTY で REPL を実行 |
| getrandom 0.4 / pbkdf2 0.13 / sha2 0.11 | `getrandom::getrandom` → `getrandom::fill` | Python `hashlib` で独立に計算した PBKDF2 のテストベクタを追加し、更新前・更新後の両方で一致（既存ユーザーのハッシュが検証できる）。salt/トークンの形式テストも追加 |
| webpki-roots 1 | なし | ビルド・TLS テスト |
| zstd 0.14 | なし | 旧バイナリ（zstd 0.13）で作ったバックアップを新ビルドで復元、逆方向も復元し、件数・合計値・個別プロパティが一致 |
| toml 1 / colored 3 | なし | クラスター設定・TLS 設定の TOML テスト |
| tokio-tungstenite | `[workspace.dependencies]` から削除（未使用） | — |

- `cargo audit`: 脆弱性 0 件、警告 3 件（bincode → task 131、encoding / unsafe-libyaml → task 132）
- `cargo update --dry-run --verbose` で残るメジャー更新は bincode / lindera（各タスク）と、axum が固定している matchit のみ
- 検証: fmt・clippy クリーン、`cargo test --workspace` 全通過（1152 件）、`bash scripts/run_e2e_local.sh` 全スイート通過

## ステータス
完了
