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

## 調査結果（2026-11-05）
CALL サブクエリに限らず、**`collect()` は常に文字列を返していた**
（`UNWIND [1,2,3] AS x RETURN collect(x)` → `"[1, 2, 3]"`）。集約の実装が要素を `format!` で文字列化して
`"[…]"` を組み立てていた。`collect(DISTINCT …)` は以前から `Value::List` を返しており、両者で型が食い違っていた。
このため `size(names)` や `names[0]` など、リストとしての後続処理も文字列として扱われていた。

## 対応内容
- `collect()` が `Value::List` を返すよう修正（null は従来どおり除外）。TCP の応答では JSON 配列になる
- maharit-viz のテーブル表示: 配列・マップのセルを `String(v)`（`Bob,Carol` / `[object Object]`）ではなく JSON で表示
- E2E `query_feature_test.py` の COLLECT チェックを「None でない」から「文字列のリスト」に強化
- conformance テストに 3 ケース追加（順序の保持、CALL サブクエリの列の collect、WITH 経由の `size()`）

## 検証
- clippy クリーン、`cargo test --workspace` 全通過、`bash scripts/run_e2e_local.sh` 全スイート通過（強化した COLLECT チェックを含む）

## ステータス
完了
