# クエリエンジン: バインディングの遅延評価

## 概要

`execute_match()` が各パターンで `Vec<Bindings>` を丸ごと作り直しているため、
複合パターン（複数の `MATCH` パターン）で中間結果が指数的に膨らむ。
イテレータベースの遅延評価に変えてメモリ使用量を削減する。

## 現状の問題

```rust
fn execute_match(&mut self, m: MatchStatement) -> Result<ResultSet, ExecuteError> {
    let mut all_bindings: Vec<Bindings> = vec![Bindings::new()];

    for pattern in &m.patterns {
        // パターンごとに全バインディングを展開・保持
        all_bindings = self.match_pattern(pattern, all_bindings)?;
        // パターン数 N、マッチ数 M のとき最大 M^N のバインディングが発生
    }
}
```

3パターン × 各100マッチ = 最大100万バインディングがメモリに展開される。

## 実装内容

- [x] `match_patterns_for_binding()` を追加してパターンチェーンを1バインディング単位で処理
  - 1つの入力バインディングを全パターンに順次チェーンする
  - 中間展開を1バインディング分に限定し、O(M^N) のピークメモリを回避
- [x] `execute_match_clause()` で `match_patterns_for_binding()` を使用してバインディング単位に処理
  - 次の入力バインディングを処理する前に中間展開を破棄
- [x] `LIMIT` 句がある場合は各セグメント後に `early_limit` で早期終了
- [x] `WHERE` フィルタをパターンマッチ直後に適用 (`execute_query_segment` 内 `retain()`)
- [x] 既存の全クエリテストが通ること

## 対象クレート

`maharit-query`
