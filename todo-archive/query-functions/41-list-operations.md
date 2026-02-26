# リスト操作

## 概要
Cypherのリスト操作（演算子・関数・内包表記）を実装する。

## 実装内容

### リスト演算子
- [x] IN演算子: value IN list
- [x] リストインデックス: list[index]
- [x] リストスライス: list[start..end]
- [x] リスト連結: list1 + list2

### リスト関数
- [x] size(list): リストの要素数
- [x] head(list): 最初の要素
- [x] last(list): 最後の要素
- [x] tail(list): 最初の要素を除いたリスト
- [x] range(start, end, step?): 数値範囲のリスト生成
- [x] reverse(list): リストの反転
- [x] reduce(accumulator = initial, variable IN list | expression): 畳み込み

### リスト内包表記
- [x] [variable IN list WHERE predicate | expression]

## クエリ例
```cypher
-- IN演算子
MATCH (n:Person) WHERE n.city IN ['Tokyo', 'Osaka', 'Kyoto'] RETURN n

-- リストインデックス
WITH ['a', 'b', 'c'] AS list RETURN list[0] AS first

-- リストスライス
WITH [1, 2, 3, 4, 5] AS list RETURN list[1..3] AS sub

-- size
MATCH (n:Person) RETURN n.name, size(n.hobbies) AS hobby_count

-- range
RETURN range(1, 10) AS numbers

-- reduce
WITH [1, 2, 3, 4, 5] AS list
RETURN reduce(total = 0, x IN list | total + x) AS sum

-- リスト内包表記
WITH [1, 2, 3, 4, 5] AS list
RETURN [x IN list WHERE x > 2 | x * 2] AS doubled
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`

## 実装完了
- 実装日: 2026-02-24
- 追加テスト: 10件 (test_in_operator, test_in_operator_null, test_list_concatenation, test_in_operator_multiple_nodes, test_size_list, test_reverse_list, test_head_last_tail, test_range, test_reduce, test_list_comprehension)
