# タスク112: 依存構成の見直し（viz の feature 化 / HTTP 重複の整理）（パッケージサイズ削減 フェーズ4）

## 概要
`maharit-server` が抱える依存を構造的に見直し、本番サーバービルドに不要なコードを除外できるようにする。

## 背景
- `maharit-server` は全 7 クレート + 約 25 個の外部クレートに依存。
- `maharit-viz` は `main.rs` の 1 経路でしか使われていない（可視化機能）。
- HTTP まわりが `axum` + `tower` + `tower-http` + `axum-server` と、独自 `http_server.rs`（軽量 tokio TCP）で**併存**している。
- 削減対象の配布バイナリは `maharit`（`maharit-server` クレート）の **1 本のみ**。依存削減の効果は `maharit-server` の依存ツリーに対してのみ評価すればよい。

## cargo-bloat 実測（.text 内訳、参考）
- tokio 928KB / std 411KB / regex(automata+syntax) 472KB / rustls+ring+aws_lc_rs 651KB / clap_builder 141KB / h2 101KB / quick_xml 77KB / maharit_viz 77KB。
- TLS・regex・HTTP スタックを feature off にできれば数 MB 削減余地あり。ただし tokio(928KB) は非同期サーバーの限り削れない。

## 対応案
1. **viz のオプション feature 化**: `maharit-viz` を `optional = true` にし `#[cfg(feature = "viz")]` でガード。
2. **HTTP スタックの整理**: `axum` 系一式と独自 `http_server.rs` のどちらに寄せるか方針決定。まず各エンドポイントの実装場所を洗い出す調査から。

## 完了条件
- [ ] viz が feature gate 化され、外した状態でサーバーがビルド・起動可能
- [ ] HTTP スタックの現状を整理した方針メモ
- [ ] 不要依存を 1 つ以上削減、または削減不可の根拠を記録
- [ ] `cargo build` / `cargo test` がパス

## 優先度
中
