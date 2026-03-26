# Task 74: エッジ作成時のノードバインディングキャッシュ

## 概要
`MATCH (a {...}), (b {...}) CREATE (a)-[...]->(b)` でのエッジ作成は
同一トランザクション内で a・b のノード検索を2回実行している。
バインド済み変数のノードID をキャッシュして2回目の検索を省略する。

## 背景（ベンチマーク根拠）
- CREATE KNOWS edges: 52/s（19 ms/op）
- CREATE nodes（7 ms）に対して 2.7倍遅い
- MATCH 2回分 + CREATE 1回 = 3ステップが原因と推定
- ノード数増加で悪化する（MATCH がフルスキャンのため）

## 実装内容

### バインディングキャッシュの活用
- [x] `execute_create_with_bindings`: 変数がバインド済みの場合ノードを再作成しない（start/end ノード両対応）
- [x] `match_node_pattern`: 変数がバインド済みの場合フルスキャンせず既存バインドを検証するのみ
- [x] 同一変数を複数パターンで参照するクエリ（`MATCH (a {...}), (a {...})`）の正確な動作を確認

### プロパティインデックスとの連携
- [x] タスク #71 で `match_node_pattern` にインデックスルックアップを実装済み
- [x] インデックス経由 MATCH → バインディング再利用 CREATE のエンドツーエンドテストを追加

### テスト（追加済み）
- [x] `test_bound_variable_reused_not_duplicated`: バインド済み変数を再利用して created_nodes=0 を確認
- [x] `test_same_variable_matched_twice_uses_cache`: 同変数2回参照で両条件 AND マッチを確認
- [x] `test_index_accelerated_match_create`: インデックス + バインドキャッシュのエンドツーエンド確認
- [x] `execute_with` ヘルパー関数を追加（同一 Executor でクエリを連続実行するテスト用）

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — `execute_create_with_bindings`、`match_node_pattern`

## ステータス
実装確認・テスト追加完了。459 tests passing。
