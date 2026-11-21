# タスク 207: 集約関数を式の中で使えるようにする

## 背景（2026-11-21、openCypher TCK）
集約関数を式の一部として書くと構文エラーになる（`expected scalar function, found aggregate function`）。

```text
RETURN count(n) + 1                       // conformance_tests の KNOWN_UNSUPPORTED にも載っている
RETURN size(collect(x)) AS n
WITH [x IN collect(n.name) | toUpper(x)] AS names RETURN names
```

TCK: `list/List11 [3]`、`List12 [3] [4] [5]`、`conformance_tests.rs` の `KNOWN_UNSUPPORTED`。

## 進め方
- `ReturnItem::Aggregate` と `Expression` が分かれていて、集約は RETURN / WITH の項目の一番外側にしか書けない
- 式の木の中の集約を見つけてグループごとに先に計算し、その値を使って残りの式を評価する形にする（グループのキーは集約を含まない項目）
- `count(n) + count(m)` のように 1 つの項目に複数の集約がある場合、集約の中に集約がある場合（エラー）も扱う

## 作業
- [ ] パーサー: 式の中の集約関数を受け付ける
- [ ] 実行: 項目ごとに集約を取り出して計算し、式に戻す
- [ ] 集約の入れ子はエラー（TCK の `InvalidAggregation`）
- [ ] docs の集約関数のページに例を足す（日英）
- [ ] `feat(query):`
