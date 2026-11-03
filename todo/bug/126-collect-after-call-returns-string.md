# タスク 126: CALL サブクエリの結果列を collect() すると文字列になる

## 概要（2026-11-03、task 123 の作業中に発見。変更前のコードでも再現）

CALL サブクエリが返したスカラー列を外側の RETURN で `collect()` すると、
リストではなくリストを文字列化した値（`"[\"Bob\"]"`）が返る。

## 再現

```cypher
MATCH (p:Person)
CALL { WITH p MATCH (p)-[:KNOWS]->(f) RETURN f.name AS name }
RETURN p.name, collect(name) AS linked
```

- 期待: `["Alice", ["Bob"]]` など（`linked` は List）
- 実際: `["Alice", "[\"Bob\"]"]`

## 調査の手がかり
- `build_result_set` の集約で、`BindingValue::Scalar` の列を集めるときの変換を確認する。
