# タスク111: panic=abort と依存 feature の絞り込み（パッケージサイズ削減 フェーズ3）

> **進捗メモ（2026-06-21）**: `panic = "abort"` は task109/110 と同時に `[profile.release]` へ適用済み（`catch_unwind`/panic hook 未使用を確認）。
> サイズ削減の本丸は task113（埋め込み IPADIC 辞書のオプション化）で、既定バイナリは **2.62MB** を達成済み。
> **本タスクの残作業は tokio 等の feature 絞り込みのみ**（既に 2.62MB のため追加効果は小。優先度低）。

## 概要
`panic = "abort"` でアンワインドコードを除去し、`tokio` 等の過剰 feature を実使用分に絞ってバイナリと依存を縮小する。

## 背景
- `tokio` が `features = ["full"]` だが、実使用は `net / io-util / sync / time / fs / signal / macros / rt-multi-thread` 程度。
- `panic = "unwind"`（デフォルト）はアンワインドテーブルを含む。
- 削減対象の配布バイナリは `maharit`（`maharit-server` クレート）の **1 本のみ**。

## 対応案
1. `[profile.release]` に `panic = "abort"`（**適用済み**）。
2. `tokio` の feature を限定:
   ```toml
   tokio = { version = "1", default-features = false, features = [
     "rt-multi-thread", "net", "io-util", "sync", "time", "fs", "signal", "macros",
   ] }
   ```
3. 余地があれば `regex` などその他の重い依存も `default-features = false` + 必要分に絞る。

## 完了条件
- [x] `panic = "abort"` 設定後も全クレートがビルド可能
- [x] `catch_unwind` / `#[should_panic]` への影響を確認（catch_unwind/panic hook 未使用を grep 確認）
- [x] tokio feature 限定後に `cargo build` / `cargo test` がパス（workspace 全16バイナリパス）
- [x] サイズ削減量を記録（下記）

## 対応（完了）
- workspace の tokio を `features = ["full"]` → `default-features = false` +
  実使用のみ（`rt-multi-thread / net / io-util / sync / time / signal / macros`）に変更。
  使用実績を grep で確認（`fs` / `process` は未使用）。
- 依存クレートが必要とする feature は cargo の feature 統合で自動有効化されるため
  破壊なし。全クレート build + workspace 全16テストバイナリパス。

## サイズ計測（release, maharit）
- 変更前: 2,799,648 bytes
- 変更後: 2,799,536 bytes（**-112 bytes ≈ 0.00%**）
- → タスク記載どおり効果は微小。cargo の feature 統合で依存が必要とする tokio 機能は
  残り、かつ LTO で未使用コードは元々除去されるため。**価値はサイズより衛生面**
  （feature の明示化・クリーンビルド時の tokio コンパイル削減）。

## メリット / デメリット
- メリット: feature 限定で依存コンパイル量とビルド時間も減少。依存の意図が明確化。
- デメリット: tokio は将来の機能追加時に feature 不足でビルドエラーになりうる（メンテコスト）。

## 優先度
低（panic=abort 部分は完了、feature 絞りのみ残） → **完了**

## ステータス
完了
