# タスク 179: E2E のクエリ検査を「期待する結果と一致するか」に揃える

## 背景（2026-11-16）
bug/176（`WHERE NOT a = b` が 0 件）は E2E に同じ形のテストがあったのに検出できなかった。

```python
# scripts/query_feature_test.py:150
"MATCH (n:Person) WHERE NOT n.city = 'Tokyo' RETURN n.name"
check(..., "Alice" not in names and "Charlie" not in names)   # 0 件でも通る
```
WHERE がエラーを 0 件に変えてしまう動作（bug/177）と組み合わさり、壊れていてもグリーンになっていた。

## 作業
- [x] scripts/ の検査で「含まれない」「1 件以上ある」だけを見ているものを洗い出す
      （`not in names` / `len(rows) > 0` / `len(rows) >= N` など。2026-11-16 時点で 17 か所。
      query_feature_test.py / constraint_test.py / auth_test.py / failover_test.py）
- [x] クエリの結果を確かめる検査は、期待する集合（順序が決まるものはリスト）との一致に変える
- [x] 例外として「1 件以上」で正しいもの（件数が決まらないもの）はコメントで理由を書く
- [x] 結果がエラーかどうか（`type == "error"`）も確かめ、エラーを 0 件扱いで通さない
- [x] `bash scripts/run_e2e_local.sh` で全項目グリーンを確認

## 結果（2026-11-21）
- 「含まれない」「1 件以上」だけを見ていた 13 か所（query_feature_test 7、constraint_test 4、auth_test 1、failover_test 3）を、期待する集合・件数との一致に変えた
- 「1 件以上」で正しいもの（件数が決まらないもの）は残っていなかった
- 結果を確かめる箇所で `type == "result"` も確かめるようにした（エラーを 0 件扱いで通さない）
- 書き換えで、前のテストが足したデータ（test_merge の Frank / Eve）が後の検査に効いていることが分かったので、期待値にコメントで書いた
- `bash scripts/run_e2e_local.sh` で全スイート通過
