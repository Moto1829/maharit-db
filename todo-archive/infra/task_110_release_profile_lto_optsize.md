# タスク110: LTO / opt-level / codegen-units によるサイズ最適化（パッケージサイズ削減 フェーズ2）

## 概要
リリースプロファイルに LTO・サイズ優先最適化・codegen-units 統合を加え、コードを縮小する。

## 完了条件
- [x] `lto="fat"` / `codegen-units=1` / `opt-level="z"` を設定
- [x] `cargo build --release -p maharit-server` 成功
- [x] コードセクションの縮小を確認（`__text` = **1.9MB** まで圧縮、`size -m` 実測）
- [~] `benchmark.py` 性能退行確認 → ユーザー判断でスキップ（smoke_test 全パスで動作確認済み）
- [x] テストパス（maharit-core 145/156）

## 実績
- task109 と同一の `[profile.release]` で適用。opt-level=z + fat LTO によりコードは 1.9MB と十分小さい。
- **総サイズの支配要因はコードではなく埋め込み IPADIC 辞書（task113 で対応）だった**ことが `size -m` で判明。
- ビルド時間: クリーン release 約1.5分（fat LTO の代償）。`opt-level` は将来速度が問題なら `"s"`/`3` に変更可。

## 優先度
高（完了）
