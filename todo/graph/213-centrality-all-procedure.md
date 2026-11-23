# タスク 213: 中心性をまとめて返す db.centrality.all

## 背景（2026-11-23、task127 の残り）
task127 で docs から外したプロシージャ。task172 で `db.pageRank` / `db.betweennessCentrality` / `db.closenessCentrality` は
個別に実装済みだが、まとめて返すものは無い。

※ 実装するかどうかは後で決める。決めるまでは着手しない。既存の 3 つを順に呼べば代わりになるので、必要性も含めて判断する。

## 候補
- `CALL db.centrality.all([config]) YIELD node, pageRank, betweenness, closeness`

## 作業
- [ ] 設定マップ（共通の `nodeLabels` / `relationshipTypes` と、pageRank の `iterations` など）の受け方を決める
- [ ] 一時 `Graph` への写しを 1 回で済ませ、3 つの計算に使い回す（`executor/algo.rs`）
- [ ] テスト、docs/algorithms/centrality.md（日英）の表に追加
