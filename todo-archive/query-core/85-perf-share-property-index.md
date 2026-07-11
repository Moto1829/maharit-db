# query-core/85: プロパティ索引をリクエスト間で共有（性能改善 5）

## 概要
`tcp_server` の `execute_query` 系は毎リクエストで
`new_concurrent_with_managers(graph, cm, fm)` を呼び、この関数は毎回
`PropertyIndex::new()`（空）を生成していた。`into_managers()` も cm/fm しか
返さないため、`CREATE INDEX` で作った B-tree 索引はリクエスト終了時に破棄され、
次のクエリでは `has_index` が常に false → 等価/範囲の索引探索経路が
ネットワーク経路では一度も発火しない（索引が実質デッド）状態だった。

## 対応
- `Executor::new_concurrent_with_managers` に `property_index: PropertyIndex` 引数を追加。
- `Executor::into_managers()` を `(ConstraintManager, FulltextManager, PropertyIndex)` に変更。
- `PropertyIndex` に `Clone` を derive（全フィールドが Clone 可能）。
- `TcpServer` に共有 `property_index: Arc<Mutex<PropertyIndex>>` を追加し、
  全コンストラクタ・`handle_connection`・3 実行関数へ配線。
  cm/fm と同様に「clone-in → execute → 成功時 write-back」で永続化。
- 統合テスト追加: `CREATE INDEX` 後、別リクエストの `SHOW INDEXES` が索引を返すこと。

## 効果
- `CREATE INDEX` した等価点探索がリクエストをまたいで有効になり、
  索引経路（O(log n) 相当）とこの後の範囲プッシュダウン（query-core/88 予定）が
  初めて実効性を持つ。

## ステータス
完了（core 150 / query 501 / server 239 パス）
