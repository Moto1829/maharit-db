# タスク: UNWIND でJSONエンコードされたマップリストがパースエラーになる

## 概要

`benchmark.py` の `bench_unwind_batch_create()` が常に 0 ops を返す。
`UNWIND` クエリに Python の `json.dumps()` でシリアライズされたマップリスト
（`[{"id": 0, "name": "Alice"}]` のような形式）を渡すと、パーサーがエラーを返す。

## 失敗したテスト

- スクリプト: `scripts/benchmark.py`
- 症状: `UNWIND batch CREATE (map list)` が 0 ops/s を記録
- エラーメッセージ:
```
{'type': 'error', 'message': 'Parse error: unexpected token: expected identifier, found "id" at Span { start: 9, end: 13, line: 1, column: 10 }'}
```

## 再現クエリ

```python
# benchmark.py の生成クエリ（json.dumps で {"key": value} 形式になる）
items = [{"id": 0, "name": "Alice0", "city": "Tokyo"}, {"id": 1, "name": "Bob1", "city": "Osaka"}]
items_json = json.dumps(items)  # -> '[{"id": 0, "name": "Alice0", "city": "Tokyo"}, ...]'
query = f"UNWIND {items_json} AS item CREATE (:UnwindBench {{id: item.id, name: item.name}})"
# ↑ このクエリでパースエラー
```

正常動作するパターン（Cypher ネイティブ記法）:
```cypher
UNWIND [{id: 0, name: 'Alice'}, {id: 1, name: 'Bob'}] AS item
CREATE (:UnwindBench {id: item.id, name: item.name})
```

## 根本原因の分析

`maharit-query` の `lexer.rs` / `parser.rs` におけるマップリテラルのパース処理に問題がある。

Cypher では Map リテラルのキーは `identifier` でも `string_literal` でも有効であるべき。
例: `{id: 1}` も `{"id": 1}` も両方有効なはず。

しかし、現在のパーサーはクォート付きキー (`"id"`) を `identifier` として認識せず、
`expected identifier, found "id"` というエラーを出す。

関連ファイル:
- `maharit-query/src/lexer.rs`: トークン種別の定義と文字列リテラルの処理
- `maharit-query/src/parser.rs`: マップリテラルのパース（`parse_map_literal()` または相当する処理）

## 対応方針

1. `maharit-query/src/parser.rs` のマップリテラルパース箇所を特定する
2. Map キーとして `StringLiteral` トークンも受け入れるように修正する
   - キーが `StringLiteral` の場合はクォートを除去して identifier として扱う
3. テストケースを追加する:
   ```rust
   // クォート付きキーのマップリテラル
   "UNWIND [{\"id\": 0, \"name\": \"Alice\"}] AS item RETURN item.id"
   ```
4. `scripts/benchmark.py` の `bench_unwind_batch_create()` が正常に動作することを確認する

## 優先度

HIGH

## 状態

完了 (2026-03-30)

## 対応内容

- `parse_properties()` で `TokenKind::String` をマップキーとして受け入れる実装は既に存在していた
- executor の `Expression::Map` 評価・`Expression::Property` の `Value::Map` アクセスも実装済みだった
- 不足していた executor レベルの統合テスト (`test_unwind_inline_json_map_create`) を追加して動作を確認・固定した

## 関連ファイル

- `maharit-query/src/parser.rs`
- `maharit-query/src/lexer.rs`
- `maharit-query/src/ast.rs`
- `scripts/benchmark.py` (bench_unwind_batch_create 関数)
