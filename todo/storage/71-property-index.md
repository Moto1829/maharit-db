# プロパティインデックス（CREATE INDEX 構文）

## 概要
`WHERE n.id = X` などのポイントルックアップが現状 O(n) フルスキャンになっている。
プロパティインデックスを作成・利用できるようにして O(log n) に改善する。

## 背景（ベンチマーク根拠）
- Repeated point-lookup (id filter): 1kノード → 13 ms、10kノード → 21 ms（線形増加）
- インデックスが効けば 0.1 ms 以下が期待できる
- スキーマ制約（UNIQUE）は実装済みだが、検索用インデックスとは別

## 実装内容

### DDL 構文
- [ ] `CREATE INDEX ON :Label(property)` のパーサー対応
- [ ] `DROP INDEX ON :Label(property)` のパーサー対応
- [ ] `SHOW INDEXES` のパーサー対応

### インデックス構造
- [ ] `IndexManager` を `maharit-core` または `maharit-storage` に実装
  - BTreeMap<Value, Vec<NodeId>> で範囲検索も対応
- [ ] ノード作成・更新・削除時にインデックスを自動更新

### クエリプランナーへの統合
- [ ] `maharit-query/src/executor.rs` の MATCH WHERE 処理でインデックスを優先利用
- [ ] `EXPLAIN` でインデックス使用有無を表示

### 永続化
- [ ] バックアップ・リストア時にインデックス定義を保存・復元

## 関連ファイル
- `crates/maharit-core/src/` — グラフ構造・LabelIndex
- `crates/maharit-query/src/executor.rs` — MATCH 処理
- `crates/maharit-query/src/planner.rs` — クエリプラン
- `crates/maharit-storage/src/` — 永続化

## ステータス
未着手
