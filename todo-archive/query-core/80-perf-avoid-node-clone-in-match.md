# query-core/80: MATCH スキャンでの Node 全体クローンを排除（性能改善 1/4）

## 概要
`GraphBackend::get_node` は `Option<Node>`（所有値）を返す設計のため、
`Graph`/`ConcurrentGraph` ともにノード全体を複製する（labels の `Vec<String>`
確保 + Arc 参照カウント操作）。この複製が `node_matches_pattern` で
**スキャン中のノードごとに発生**しており、全 MATCH のホットパスを重くしていた。

## 対応
- `GraphBackend` にノード全体をクローンしない狭い読み取りメソッドを追加
  （オブジェクト安全を保つため非ジェネリック）:
  - `node_has_all_labels(id, labels) -> bool`
  - `get_node_property(id, key) -> Option<PropertyValue>`（単一プロパティのみ複製）
- 両バックエンド（Graph は `get_node().has_label`、ConcurrentGraph は `with_node`）で実装。
- `executor.rs::node_matches_pattern` を書き換え、ラベル判定・プロパティ取得を
  上記メソッド経由に変更。ラベルのみの MATCH で Node 全体の複製が不要になった。

## 効果
- ラベルフィルタ主体の MATCH（`MATCH (n:Label)`）でノードごとの Vec 確保が消える。
- 大規模ノードで効果が拡大（ベンチ項目追加で可視化予定）。

## ステータス
完了（core 145 / query 497 テストパス）
