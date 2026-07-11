# query-core/81: ラベル索引を MATCH スキャンに配線（性能改善 2/4）

## 概要
`MATCH (n:Person)` でも executor は `node_ids()`（全ノード）を取得してから
1件ずつラベル判定していた。コアの `LabelIndex` は index.rs に存在するのに
Graph/ConcurrentGraph に統合されておらず未使用だった。

## 対応
- `Graph` / `ConcurrentGraph` に `label → NodeId 集合` の索引を追加し、
  作成・削除・ラベル増減で維持する（永続化はロード時の create 経由で再構築されるため
  シリアライズ対象外）。
  - `Graph`: `HashMap<String, HashSet<NodeId>>` を追加。
    `create_node_with_labels` / `create_node_with_id_and_labels` / `delete_node` を更新。
    索引を維持する `add_node_label` / `remove_node_label` / `nodes_by_label` を追加。
  - `ConcurrentGraph`: `Arc<DashMap<String, HashSet<NodeId>>>` を追加し同様に維持。
- `GraphBackend` に `nodes_by_label(label) -> Vec<NodeId>` を追加し、両バックエンドの
  `add_node_label` / `remove_node_label` を索引更新版に差し替え。
- `executor.rs::match_node_pattern`: ラベル指定パターンは `nodes_by_label` で
  候補を該当ラベルのノードのみに絞る。追加ラベル・プロパティは `node_matches_pattern`
  が引き続き検証。

## 効果
- `MATCH (n:Label)` が O(全ノード) → O(該当ラベルのノード)。
  多ラベル・選択的ラベルの大規模グラフで効果大。
- ラベル変更を伴う全経路（SET/REMOVE label, create/delete）が索引更新版を通ることを確認。
  `get_node_mut` の直接ラベル変更バイパスは存在しない（プロパティ設定のみ）。

## ステータス
完了（core 150 / query 497、workspace 全16バイナリパス）
