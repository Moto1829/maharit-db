# タスク109: リリースプロファイル設定（パッケージサイズ削減 フェーズ1）

## 概要
`[profile.release]` が未設定でデフォルトのままのため、デバッグシンボルを含んだまま配布される。strip 等を追加してバイナリサイズを削減する。

## 背景
- `target/release/maharit` は原状 **24.2 MB**（unstripped）。
- 配布バイナリは `maharit`（`maharit-server`）の **1 本のみ**。`[profile.release]` は workspace 全体に適用される。

## 完了条件
- [x] `[profile.release]` に `strip = true` ほかを追加
- [x] `cargo build --release -p maharit-server` 成功
- [x] サイズ削減を確認（**24.2MB → 18.0MB**、strip 済みを `size -m` で確認）
- [x] smoke_test 全パス

## 実績（クリーンビルド実測）
- `strip + lto=fat + codegen-units=1 + opt-level=z + panic=abort` を適用し **24.2MB → 18.0MB**。
- 内訳: `__text`(コード) 1.9MB / `__const` 16.5MB。**残り 16.5MB の大半は埋め込み IPADIC 辞書**で、これは task113 で別途オプション化した。

## 優先度
高（完了）
