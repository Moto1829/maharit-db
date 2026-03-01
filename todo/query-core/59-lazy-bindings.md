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

- [ ] `match_pattern()` の戻り値をイテレータ（`impl Iterator<Item = Bindings>`）に変更
- [ ] パターンチェーンを `flat_map` で遅延結合
  ```rust
  let bindings_iter = m.patterns.iter().fold(
      Box::new(std::iter::once(Bindings::new())) as Box<dyn Iterator<Item = Bindings>>,
      |acc, pattern| Box::new(acc.flat_map(|b| self.match_pattern_iter(pattern, b)))
  );
  ```
- [ ] `LIMIT` 句がある場合は `take(limit)` で早期終了（現状は全件マッチ後に切り捨て）
- [ ] `WHERE` フィルタをパターンマッチ直後に適用してイテレータを絞る
- [ ] 既存の全クエリテストが通ること

## 期待効果

- 複合パターンクエリのメモリ使用量 -60%
- `LIMIT` 付きクエリの速度向上（全件展開が不要になる）

## 対象クレート

`maharit-query`
