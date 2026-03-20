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
- [x] `CREATE INDEX ON :Label(property)` のパーサー対応
- [x] `DROP INDEX ON :Label(property)` のパーサー対応
- [x] `SHOW INDEXES` のパーサー対応

### インデックス構造
- [x] `PropertyIndex` を `maharit-core` に実装済み（BTreeMap ベースの範囲検索対応）
- [x] ノード作成時にインデックスを自動更新（`create_node()` 内）

### クエリプランナーへの統合
- [x] `executor.rs` の `match_node_pattern()` でインデックス優先ルックアップを実装
- [x] `planner.rs` に `CreateIndex`/`DropIndex`/`ShowIndexes` プランノードを追加

### 永続化
- [ ] バックアップ・リストア時にインデックス定義を保存・復元（未着手）

## 関連ファイル
- `crates/maharit-core/src/property_index.rs` — PropertyIndex 実装
- `crates/maharit-query/src/ast.rs` — Statement enum / CreateIndexStatement / DropIndexStatement
- `crates/maharit-query/src/parser.rs` — parse_create_index / parse_drop_index / parse_show
- `crates/maharit-query/src/executor.rs` — execute_create_index / match_node_pattern（インデックス最適化）
- `crates/maharit-query/src/planner.rs` — CreateIndex/DropIndex/ShowIndexes プランノード

## ステータス
実装完了（永続化は除く）。452 tests passing。
