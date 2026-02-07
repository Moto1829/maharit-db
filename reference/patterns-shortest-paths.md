# Shortest Paths

Source: https://neo4j.com/docs/cypher-manual/current/patterns/shortest-paths/

## 概要
- `SHORTEST` は最短路（ホップ数）を返す。
- `SHORTEST`は `shortestPath()` / `allShortestPaths()` を置き換える（関数は非GQL準拠）。

## 主なセレクタ
- `SHORTEST k`: 最短からk本
- `ALL SHORTEST`: 最短長に同長なパスすべて
- `SHORTEST k GROUPS`: 最短長のグループをk段階まで
- `ANY`: 到達可能性を示す（`SHORTEST 1`と同等の意味）

## パーティション
- 複数の始点/終点がある場合、開始/終了の組合せごとにパーティション化して最短路を選択。

## フィルタ
- インライン `WHERE` は**事前フィルタ**。
- `MATCH` 後の `WHERE` は**事後フィルタ**。
- 事前フィルタにするにはパターン内 `WHERE` または括弧で囲む。

## 性能/プラン
- 単一 source-target の特定は `CALL` サブクエリや一意制約で補助できる。
- オペレーター: `ShortestPath`, `StatefulShortestPath(Into)`, `StatefulShortestPath(All)`。

## GDSとの使い分け
- 重み付きや特定アルゴリズムはGDS推奨。
- 複雑なパスパターンはCypherが適する。
