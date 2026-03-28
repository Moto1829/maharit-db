# タスク: RETURN 単独ステートメントが未サポート

## 概要

Cypher では `RETURN 1 + 1 AS result` のような、`MATCH` を伴わない単独 `RETURN` ステートメントが
有効な構文である。しかし現在の maharit では未サポートであり、パースエラーになる。

## 失敗したテスト

- スクリプト: 手動確認（`smoke_test.py` / `benchmark.py` の間接的な影響）
- エラーメッセージ:
```
{'type': 'error', 'message': 'Parse error: unexpected token: expected CREATE, MATCH, MERGE, UNWIND, FOREACH, DROP, DROP INDEX, SHOW, ALTER, CALL, EXPLAIN, or PROFILE, found RETURN at Span { start: 0, end: 6, line: 1, column: 1 }'}
```

## 再現クエリ

```cypher
RETURN 1 + 1 AS result
RETURN 'hello' AS greeting
RETURN true AND false AS check
```

## 根本原因の分析

`maharit-query/src/parser.rs` の `parse()` 関数がトップレベルのディスパッチで
`RETURN` トークンを認識していない。`parser.rs` は `Statement` enum の値として
`RETURN` 単体を持つ構造になっていない可能性がある。

`ast.rs` の `Statement` enum に `Return` バリアントが存在しないか、
`parser.rs` の `parse()` 関数が `Token::Return` をディスパッチしていない。

## 対応方針

1. `maharit-query/src/ast.rs` に `Statement::Return(Vec<ReturnItem>)` を追加（既存なら確認）
2. `maharit-query/src/parser.rs` の `parse()` に `Token::Return` のケースを追加
3. `maharit-query/src/executor.rs` で `Statement::Return` を実行する処理を追加
   - `WITH` 節の実行に類似したロジックで `ReturnItem` を評価して返す
4. テストケースを追加する

## 優先度

MEDIUM

## 関連ファイル

- `maharit-query/src/ast.rs`
- `maharit-query/src/parser.rs`
- `maharit-query/src/executor.rs`
