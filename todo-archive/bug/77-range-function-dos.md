# bug/77: range() 関数のリソース枯渇 DoS

## 概要

`maharit-query` の `range(start, end, step)` スカラー関数に上限がなく、任意の読み取り
クライアントがメモリ枯渇・CPU 無限ループを引き起こせる DoS 脆弱性。

`crates/maharit-query/src/executor.rs` の `ScalarFunction::Range` 実装
（`ScalarFunction::Range(start_expr, end_expr, step_expr)`）は結果リストを無制限に
`Vec` へ push していた。

## 攻撃例

- **メモリ枯渇**: `RETURN range(0, 9000000000000000000)` は約 9×10^18 要素を確保しようとし、
  即座に OOM を誘発する。読み取り専用ロールでも実行可能。
- **無限ループ (CPU/メモリ)**: `RETURN range(9223372036854775806, 9223372036854775807, 2)`
  はカウンタ `i += step` が i64 オーバーフローで負値へラップし、
  release ビルドでは `while i <= end` が永久に真になり無限ループ + 無制限 push となる。

いずれもネットワーク経由の単一クエリで到達可能で、bug/70（メッセージサイズ上限）と
同カテゴリのリソース枯渇 DoS。

## 修正

1. リストを確保する前に要素数を i128 で事前計算し、上限 `MAX_RANGE_LEN`
   （10,000,000 要素）を超える場合は `TypeError` で拒否する（巨大確保の防止）。
2. カウンタ加算を `checked_add` にして、オーバーフロー時はループを終了する
   （無限ループの防止）。

## 影響範囲

- `crates/maharit-query/src/executor.rs`: `ScalarFunction::Range` 実装のみ。
- 正常系（昇順/降順/step 指定/空範囲）の挙動は不変。

## ステータス
完了（`crates/maharit-query/src/executor.rs` に `MAX_RANGE_LEN=10_000_000` +
`checked_add` を実装。コミット `345f04f3` に混入して反映済み。テスト3件追加）。
