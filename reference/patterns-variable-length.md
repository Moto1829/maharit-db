# Variable-length Patterns

Source: https://neo4j.com/docs/cypher-manual/current/patterns/variable-length-patterns/

## Quantified path patterns
- 繰り返し部分を `((...)){m,n}` で表現し、未知/範囲長のパスに一致。
- `{1,3}` のように範囲指定。
- 量指定パターンは `UNION` の代替として可変長を単一クエリで表現。

## Quantified relationships
- 量指定パスの簡略記法。
- 例: `-[:NEXT]->{1,10}` のように関係パターンに量指定。
- 量指定のスコープは関係パターンのみ。

## Group variables
- 量指定パス内で宣言された変数は**リスト（グループ変数）**として外側で参照。
- グループ変数は list comprehension や `reduce()` と相性が良い。

## Predicates in quantified path patterns
- 内部 `WHERE` で探索空間を制限。
- 例: 関係の距離やノードラベルなど。
- `allReduce()` を使うと累積値条件で枝刈り可能。

## 参考
- Quantified relationships: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#quantified-relationships
- Variable-length relationships: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#variable-length-relationships
