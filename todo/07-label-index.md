# ラベルインデックス

## 概要
ノードラベルによる高速な検索を可能にするインデックスを実装する。

## 実装内容

### インデックス構造
- [x] ラベル -> ノードID集合のマッピング
- [x] HashMapベースの実装

### インデックス更新
- [x] ノード作成時の自動インデックス追加
- [x] ノード削除時の自動インデックス削除
- [ ] ラベル変更時の更新（将来的に）

### クエリAPI
- [x] `get_nodes_by_label(label: &str) -> Vec<NodeId>`
- [x] `count_nodes_by_label(label: &str) -> usize`

### エッジラベルインデックス
- [x] エッジタイプ -> エッジID集合のマッピング
- [x] `get_edges_by_type(edge_type: &str) -> Vec<EdgeId>`

## 対象クレート
`maharit-core`
