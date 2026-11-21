# バグ 203: STARTS WITH / ENDS WITH / CONTAINS が大文字小文字を区別しない

## 現状（2026-11-21、openCypher TCK で発見）
`executor/expr.rs` の `apply_binary_op` が両辺を `to_lowercase()` してから比べている。
標準の Cypher は大文字小文字を区別する（`'ABCDEF' STARTS WITH 'abc'` は false）。

```text
RETURN 'ABCDEF' STARTS WITH 'abc' AS x   // => true   期待: false
RETURN 'ABCDEF' CONTAINS 'cde' AS x      // => true   期待: false
RETURN 1 STARTS WITH 'a' AS x            // => type error   期待: null
```

TCK: `string/String8` `String9` `String10` `String11` の 12 シナリオ（`[1] [2] [8] [9]` など）。

## 注意
- `CONTAINS` には独自の拡張がある（`'"a b"'` で語の並び、`'term~2'` であいまい検索、`string_contains`）。
  標準では `'say "a b"' CONTAINS '"a b"'` は引用符を含む部分文字列の検索なので、拡張と衝突する。
  拡張は全文索引のプロシージャ（`db.index.fulltext.search`）へ移すか、別の関数にするかを決める
- docs（`docs/cypher/where.md` の文字列述語、`docs/functions/fulltext.md`）が大文字小文字を区別しないと書いていれば直す

## 作業
- [ ] 大文字小文字を区別する。どちらかが文字列でなければ null（型エラーにしない）
- [ ] `CONTAINS` の独自拡張の扱いを決めて移す
- [ ] TCK の該当シナリオを `tck_known_failures.txt` から外す（テストが外せと知らせる）
- [ ] 結果が変わるので `fix(query)!:`
