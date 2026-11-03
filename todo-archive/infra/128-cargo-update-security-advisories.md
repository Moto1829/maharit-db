# タスク 128: cargo update で既知の脆弱性を解消する（優先度: 高）

## 概要（2026-11-03 の `cargo audit` 結果）

ワークスペースの `Cargo.lock` に RustSec の脆弱性が **36 件**、unsound / メンテ終了などの警告が 21 件ある。
そのうち脆弱性 34 件と unsound 警告すべては、互換範囲内の `cargo update` だけで解消できる。
`Cargo.lock` のコピーで `cargo update` → `cargo test --workspace` が全件通ることを確認済み。

## 既定ビルド（リリースバイナリ）に入っている主なもの

| クレート | 現在 | 主な内容 | 影響する箇所 |
|---|---|---|---|
| rustls | 0.23.36 | TLS 1.3 で暗号化レベルをまたいだハンドシェイクメッセージを受理、`Debug` で秘密情報が出る など 4 件 | TLS サーバー / クライアント |
| rustls-webpki | 0.103.9 | 信頼されていない中間 CA を受理、名前制約の誤判定、CRL 解析での panic など 5 件 | 証明書検証 |
| aws-lc-sys | 0.37.0 | PKCS7 検証のバイパス（high）、CRL 判定の誤り（high）など 5 件 | rustls の暗号バックエンド |
| webpki-roots | 1.0.6 | 失効した emSign 中間 CA を信頼してしまう | クライアントの既定ルート証明書 |
| h2 | 0.4.14 | 空の DATA フレームを際限なく受け付ける（DoS） | maharit-viz の HTTP |
| zstd / zstd-safe / zstd-sys | 0.13.3 / 7.2.4 / 2.0.16 | 再利用コンテキストへの前のストリームのデータ漏れ、UAF など多数 | バックアップの zstd 圧縮 |
| bytes / tokio / crossbeam-epoch | 1.11.0 / 1.49.0 / 0.9.18 | 整数オーバーフロー、I/O safety 違反など | 全般 |

`japanese` feature（lindera）経由のもの（rkyv、tar、quinn-proto、memmap2 など）も同じ更新で解消する。

## 作業
1. `cargo update`
2. `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`bash scripts/run_e2e_local.sh`
3. `cargo audit` で残りが quick-xml（task 129）とメンテ終了の警告（task 130〜132）だけになったことを確認

## 補足
- `cargo audit` はローカルに無かったため、`cargo install cargo-audit --locked --root target/tools` で入れて
  `target/tools/bin/cargo-audit audit` で実行した。CI への組み込みは task 135。

## 対応内容（2026-11-03）
- `cargo update` を適用（`Cargo.lock` のみの変更）
- `cargo audit`: 脆弱性 36 件 → 2 件（quick-xml。task 129）、警告 21 件 → 4 件（メンテ終了の bincode /
  encoding / rustls-pemfile / unsafe-libyaml。task 130〜132）。unsound の警告はすべて解消
- 検証: `cargo fmt --check`・`cargo clippy --workspace --all-targets -- -D warnings` クリーン、
  `cargo test --workspace` 全通過（1144 件）、`bash scripts/run_e2e_local.sh` 全スイート通過

## ステータス
完了
