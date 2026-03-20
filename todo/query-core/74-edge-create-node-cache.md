# エッジ作成時のノードバインディングキャッシュ

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
- [ ] `Executor` の `bindings: HashMap<String, BindingValue>` に既にバインド済みの変数を再利用
- [ ] `MATCH (a:Label {prop: val})` で a をバインドした後、同一クエリ内の2回目の参照では検索をスキップ
- [ ] `execute_match_create` / `execute_match_pattern` で変数再利用パスを追加

### プロパティインデックスとの連携
- [ ] タスク #71（プロパティインデックス）実装後、MATCH のフルスキャンをインデックス検索に切り替える
- [ ] 両方合わさることでエッジ作成が大幅に高速化される見込み

### テスト
- [ ] 同一変数を複数回参照するクエリで正しい結果が返ることを確認
- [ ] バインディングキャッシュ有無での実行計画差異を EXPLAIN で確認

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — `execute_match_create`、バインディング管理

## ステータス
未着手
