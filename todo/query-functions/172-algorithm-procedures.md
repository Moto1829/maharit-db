# タスク 172: グラフアルゴリズムを Cypher のプロシージャとして呼べるようにする

## 背景（2026-11-14、task127 から分離）
中心性・連結成分などは `maharit-core::algorithms` の Rust API だけで、Cypher からは使えない
（docs/algorithms/centrality.md・connected-components.md に明記済み）。

## 候補
- `CALL db.pageRank({iterations, dampingFactor}) YIELD node, score`
- `CALL db.connectedComponents() YIELD node, componentId`、`db.stronglyConnectedComponents`、`db.labelPropagation`
- `db.betweennessCentrality` / `db.closenessCentrality`
- 重み付き最短経路 `db.shortestPath.dijkstra(from, to, 'weightProp')`

## 課題
- `algorithms.rs` の関数は `&Graph`（具体型）を取るが、Executor は `&dyn GraphBackend` を持つ（サーバーは ConcurrentGraph の場合あり）。
  アルゴリズムを `GraphBackend` に対して書き直すか、隣接情報のスナップショットを作って渡す
- ラベル・リレーションシップの型で対象を絞る引数
- 大きなグラフでの実行時間（クエリタイムアウトとの関係）、読み取りとして並列実行してよいか
- 実装したら docs の「Cypher からは使えません」を書き換える
