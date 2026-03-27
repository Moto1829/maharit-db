# Task 85: ROLLBACK が実際に変更を取り消すよう修正する

## ステータス: 完了

## 内容
tcp_server.rs でクエリ実行時に tx_id が無視されており、Executor がアンドゥログに書き込まないため ROLLBACK が機能していなかった。

## 実装済み内容
- transaction.rs: CreateEdge/DeleteEdge/SetEdgeProperty を UndoRecord に追加、外部記録用 public API 追加
- tcp_server.rs: execute_query_with_tx() でスナップショット→diff→アンドゥログ記録
- smoke_test.py: ROLLBACK の厳密チェックを復活（35/35 通過）
