# タスク113: 埋め込み IPADIC 辞書のオプション化（パッケージサイズ削減 — 最大の施策）

## 概要
バイナリの大半を占める埋め込み日本語辞書 IPADIC をデフォルト off にし、既定の配布バイナリを劇的に小型化する。

## 背景（`size -m` 実測で判明）
- `[profile.release]` 適用後でも 18.0MB あり、その内訳は `__text`(コード) 1.9MB に対し **`__const` 16.5MB**。
- 真因は `maharit-core/Cargo.toml` の `default = ["japanese"]` → `lindera`(`embed-ipadic`)。
  **日本語形態素解析辞書 IPADIC（約15.3MB）がバイナリに丸ごと埋め込まれていた**。
- `japanese` を外すと **18.0MB → 2.62MB（-85%）** を実測。

## 対応（実施済み）
1. `maharit-core/Cargo.toml`: `default = ["japanese"]` → `default = []`（japanese は opt-in に）。
2. `maharit-core/src/fulltext.rs`: `contains_japanese` に `#[cfg(feature = "japanese")]` を付与（off 時の dead_code warning 解消）。
3. `maharit-server/Cargo.toml`: パススルー feature 追加 `japanese = ["maharit-core/japanese"]`。

## 使い方
- 既定（小型）: `cargo build --release -p maharit-server` → **2.62MB**
- 日本語FTS同梱: `cargo build --release -p maharit-server --features japanese` → 18.05MB

## 完了条件
- [x] 既定ビルドが warning/error なしで成功し 2.62MB
- [x] `--features japanese` ビルドが成功し従来通り 18.05MB・日本語トークナイズ有効
- [x] テストパス（既定 145 / japanese 156、差分は cfg ゲートされた日本語テスト11件）
- [x] 既定バイナリで smoke_test 全32件パス

## トレードオフ
- 既定ビルドでは日本語形態素解析が無効化され、全文検索は空白区切り等の簡易トークナイズになる。日本語FTSが必要なリリースは `--features japanese` でビルドする運用。

## 優先度
最高（完了）— パッケージサイズ削減の本丸
