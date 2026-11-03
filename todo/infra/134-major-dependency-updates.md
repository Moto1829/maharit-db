# タスク 134: メジャーバージョンが上がった依存の更新

## 概要（2026-11-03 の `cargo update --dry-run --verbose`）

互換範囲外で新しい版が出ている直接依存。既知の脆弱性は無い（quick-xml は task 129、lindera は task 132 で扱う）。

| クレート | 現在 | 最新 | 使っている箇所 / 注意 |
|---|---|---|---|
| axum / axum-server | 0.7 / 0.7 | 0.8 / 0.8 | maharit-viz。ルートのパス構文（`/:id` → `/{id}`）が変わる |
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
