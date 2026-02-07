# Patterns Primer

Source: https://neo4j.com/docs/cypher-manual/current/patterns/primer/

## 例示グラフ
- `Station` と `Stop` の鉄道ネットワークモデル。
- `Stop` は `CALLS_AT` で `Station` に結びつき、`NEXT` で停車順を表す。

## 固定長パスのマッチ
- 空のノードパターン `()` は全ノードに一致。
- ラベル指定でフィルタ可能（例: `MATCH (:Stop)`）。
- パスパターンは関係と接続ノードを含む。
- ノード/関係パターン内に `WHERE` をインラインで置ける。

## 可変長パスのマッチ
- 量指定関係（quantified relationship）で型を固定し可変長探索。
- 量指定パスパターン（quantified path pattern）ではノード/関係に `WHERE` を含められる。
- 量指定パス内で宣言した変数は**グループ変数（リスト）**として外側で参照される。

## 最短路のマッチ
- `SHORTEST k` / `ALL SHORTEST` で最短路を探索。
- `SHORTEST k` で上位k本の最短路。

## 参照リンク
- Fixed-length patterns: https://neo4j.com/docs/cypher-manual/current/patterns/fixed-length-patterns/
- Variable-length patterns: https://neo4j.com/docs/cypher-manual/current/patterns/variable-length-patterns/
- Shortest paths: https://neo4j.com/docs/cypher-manual/current/patterns/shortest-paths/
