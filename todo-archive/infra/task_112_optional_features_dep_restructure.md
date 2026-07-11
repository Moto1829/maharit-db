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
- [x] viz が feature gate 化され、外した状態でサーバーがビルド・起動可能
      → **既にクレート分離で達成済み**（下記調査）。maharit-server は maharit-viz を
        依存しておらず、`cargo build -p maharit-server` は常に viz なしでビルドされる。
- [x] HTTP スタックの現状を整理した方針メモ（下記）
- [x] 不要依存を 1 つ以上削減、または削減不可の根拠を記録（下記＝そもそも配布バイナリに含まれない）
- [x] `cargo build` / `cargo test` がパス（変更なしで既存が通る）

## 調査結果（2026-07・cargo tree で確認）
タスク記載の前提「maharit-server が axum + tower + tower-http + axum-server を抱える」は**古い**。
現状の配布バイナリ `maharit`（maharit-server クレート）の依存ツリーには
**axum / tower / tower-http / axum-server / tokio-tungstenite / maharit-viz / quick-xml は含まれない**
（`cargo tree -p maharit-server` で該当なし）。ソースにも `axum`/`tower` 参照なし。

### HTTP スタックの方針メモ
- **配布バイナリ（maharit-server）**: 監視エンドポイント（`/metrics`・`/health` 系）は
  独自の軽量 `http_server.rs`（tokio TCP）**のみ**。axum スタックは含まない＝**重複なし**。
- **axum 一式は `maharit-viz`（別クレート・別バイナリ）に隔離済み**。可視化 Web が不要なら
  `maharit-viz` をビルドしなければよい（既に分離されている）。
- 結論: task の目的（配布バイナリを viz/axum 依存から切り離す）は**クレート分離で既に達成**。
  maharit-server 側に feature ゲートを足す対象は存在せず、no-op になるため追加しない。

### 追加の余地（別タスク候補・任意）
- `cargo build` / `cargo test`（workspace 全体）は maharit-viz も含めて axum をコンパイルする。
  ワークスペースの `default-members` から maharit-viz を外せば**デフォルトビルド時間**を短縮できる
  （`cargo build -p maharit-viz` は引き続き可能）。ただし「既定で viz を建てない」は運用上の
  期待を変えるため、要判断。今回は範囲外。

## 優先度
中 → **完了（調査により変更不要と確認）**

## ステータス
完了
