# タスク 204: 演算子の優先順位と後置の述語（IS NULL / IN / STARTS WITH）、べき乗 `^`

## 背景（2026-11-21、openCypher TCK）
次の形が構文エラーになる。

```text
RETURN (a AND b) IS NULL = (b AND a) IS NULL AS r   // IS NULL の後ろに比較が続く
RETURN [1] IN [[1]] = true AS r                      // IN の後ろに比較が続く
RETURN x STARTS WITH 'a' AND y                       // 文字列述語と論理演算
RETURN 2 ^ 3 AS r                                    // べき乗演算子が無い（字句エラー）
```

Cypher の優先順位（高い順）: プロパティ・添字 → `^` → 単項 `-` → `*` `/` `%` → `+` `-` →
`IS NULL` / `IN` / `STARTS WITH` / `ENDS WITH` / `CONTAINS` / `=~` → 比較 `=` `<>` `<` … → `NOT` → `AND` → `XOR` → `OR`。
今のパーサーは後置の述語を比較と同じ段で 1 回だけ読んでいて、連鎖できない。

TCK: `precedence/Precedence1` `Precedence2` `Precedence3` `Precedence4` の約 55 シナリオ、`boolean/Boolean1`〜`Boolean5` の 9 シナリオ。

## 作業
- [ ] `parser/expr.rs` に後置の述語の段を比較の下に設け、繰り返し読めるようにする
- [ ] `^`（べき乗、右結合、結果は浮動小数）を字句・構文・実行に足す
- [ ] 優先順位を docs（`docs/cypher/where.md`）に表で載せる（日英）
- [ ] TCK の該当シナリオを一覧から外す
- [ ] 今まで構文エラーだったものが通るだけなら `feat(query):`。読み方が変わるものがあれば `!`
