# タスク 212: 経路探索のプロシージャ（db.shortestPath.dijkstra / db.allPaths）

## 背景（2026-11-23、task127 の残り）
task127 で docs から外し、task172 でも「重み付き最短経路のプロシージャは未実装」のまま残った。
エンジンには本体がある（`maharit-core/src/traversal.rs` の `Dijkstra` / `AStar` / `all_paths`）が、Cypher から呼べない。
今は docs/algorithms/shortest-path.md で `reduce` を使って重みを比べる方法を案内している。

※ 実装するかどうかは後で決める。決めるまでは着手しない。

## 候補
- `CALL db.shortestPath.dijkstra(from, to, 'weightProp' [, config]) YIELD path, cost`
- `CALL db.allPaths(from, to [, {maxDepth}]) YIELD path`
- （任意）`db.shortestPath.astar(from, to, 'weightProp', 'latProp', 'lonProp')`

## 作業
- [ ] 引数の形を決める（ノード / ノード ID、重みのプロパティ名、`relationshipTypes` / 向き / `maxDepth` の設定マップ）
- [ ] task172 と同じく、対象を一時 `Graph` に写すか、`GraphBackend` に対して直接たどるかを決める（`all_paths` は `&Graph` を取る）
- [ ] `executor/algo.rs` か新しいモジュールに追加し、`path` を Cypher の Path 値として返す（`relationships(p)` / `nodes(p)` が使えること）
- [ ] 重みが無い・数値でない・負のリレーションシップの扱いを決める
- [ ] 全経路の爆発を防ぐ上限（`maxDepth` 必須にする・件数上限・タイムアウト）
- [ ] ACL（見えないノード・リレーションシップを経路に含めない）
- [ ] テスト、docs/algorithms/shortest-path.md と all-paths.md（日英）に Cypher の節を追加
