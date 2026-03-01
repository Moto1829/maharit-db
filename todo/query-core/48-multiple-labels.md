# ノードの複数ラベル対応

## 概要
Neo4j 互換の複数ラベル（`(n:Person:Employee)`）をノードに付与できるようにする。
現状は `Node.label: String`（単一ラベル）のみ対応。

## 現状の問題

- `maharit-core::Node.label: String` → 単一ラベルのみ
- `LabelIndex.node_to_label: HashMap<NodeId, String>` → 逆引きが単一ラベル
- `NodePattern.label: Option<String>` → クエリパターンも単一ラベルのみ

## 実装内容

### maharit-core の変更

- [ ] `Node.label: String` → `Node.labels: Vec<String>` に変更
- [ ] `LabelIndex.node_to_label: HashMap<NodeId, String>` → `HashMap<NodeId, Vec<String>>` に変更
- [ ] `LabelIndex::get_nodes_by_label()` は既存ロジックを維持（各ラベルに対するインデックス）
- [ ] `Graph::create_node()` / `create_node_with_id()` を `labels: Vec<String>` 対応に変更
- [ ] 既存の単一ラベル API（`label: &str`）との後方互換ヘルパーを追加

### maharit-query の変更

- [ ] `NodePattern.label: Option<String>` → `NodePattern.labels: Vec<String>` に変更
- [ ] パーサー: `(n:Person:Employee)` のように `:Label` を複数連続でパースできるようにする
- [ ] エグゼキュータ: ノードが **全ての指定ラベルを持つ** 場合にマッチする（AND 条件）
- [ ] `SET n:NewLabel` でラベルを追加できるようにする（既存ラベルを保持）
- [ ] `REMOVE n:OldLabel` でラベルを削除できるようにする

### その他

- [ ] `maharit-storage` の永続化フォーマットを複数ラベル対応に更新
- [ ] 既存テストが全て通ること

## クエリ例

```cypher
-- 複数ラベルのノード作成
CREATE (n:Person:Employee {name: "Alice"})

-- 複数ラベルでのマッチ（AND 条件）
MATCH (n:Person:Employee) RETURN n.name

-- ラベルの追加
MATCH (n:Person {name: "Alice"}) SET n:Manager RETURN labels(n)

-- ラベルの削除
MATCH (n:Person:Manager {name: "Alice"}) REMOVE n:Manager RETURN labels(n)

-- ラベル一覧の取得
MATCH (n:Person) RETURN n.name, labels(n)
```

## 依存

- `03-query-parser.md` が完了していること
- `07-label-index.md` が完了していること

## 対象クレート

`maharit-core`, `maharit-query`, `maharit-storage`
