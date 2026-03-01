# リスト操作

MaharitDB は Cypher クエリ内でリストを操作するための関数と構文を提供します。

## リスト要素へのアクセス

### head(list)

リストの最初の要素を返します。空リストの場合は `null`。

```cypher
RETURN head([1, 2, 3])
-- 結果: 1

RETURN head([])
-- 結果: null
```

### last(list)

リストの最後の要素を返します。空リストの場合は `null`。

```cypher
RETURN last([1, 2, 3])
-- 結果: 3
```

### tail(list)

リストの最初の要素を除いたサブリストを返します。

```cypher
RETURN tail([1, 2, 3])
-- 結果: [2, 3]

RETURN tail([1])
-- 結果: []
```

### インデックスアクセス

角括弧でインデックス指定できます（0 始まり）。

```cypher
RETURN [1, 2, 3][0]
-- 結果: 1

RETURN [1, 2, 3][2]
-- 結果: 3

-- 負のインデックス（末尾から）
RETURN [1, 2, 3][-1]
-- 結果: 3
```

## リストのサイズ

### size(list)

リストの要素数を返します。

```cypher
RETURN size([1, 2, 3])
-- 結果: 3

MATCH (n:Person)
RETURN n.name, size(n.tags) AS tag_count
```

## リストの生成

### range(start, end[, step])

数値の連続リストを生成します。

```cypher
RETURN range(1, 5)
-- 結果: [1, 2, 3, 4, 5]

RETURN range(0, 10, 2)
-- 結果: [0, 2, 4, 6, 8, 10]

RETURN range(5, 1, -1)
-- 結果: [5, 4, 3, 2, 1]
```

## リストの変換

### reverse(list)

リストを逆順にした新しいリストを返します。

```cypher
RETURN reverse([1, 2, 3])
-- 結果: [3, 2, 1]

RETURN reverse(["a", "b", "c"])
-- 結果: ["c", "b", "a"]
```

## リストスライス

`[start..end]` 構文でリストの部分を取得します。

```cypher
RETURN [1, 2, 3, 4, 5][1..3]
-- 結果: [2, 3]

-- 最初から n 個
RETURN [1, 2, 3, 4, 5][..3]
-- 結果: [1, 2, 3]

-- n 番目以降
RETURN [1, 2, 3, 4, 5][2..]
-- 結果: [3, 4, 5]
```

## reduce

`reduce` はリストを順に処理して単一の値にまとめます。

```cypher
-- リストの合計
RETURN reduce(total = 0, x IN [1, 2, 3, 4, 5] | total + x)
-- 結果: 15

-- リストの最大値
RETURN reduce(m = 0, x IN [3, 1, 4, 1, 5, 9, 2, 6] | CASE WHEN x > m THEN x ELSE m END)
-- 結果: 9

-- 文字列の結合
RETURN reduce(s = "", name IN ["Alice", "Bob", "Charlie"] | s + name + ", ")
-- 結果: "Alice, Bob, Charlie, "
```

## リスト内包表記

`[variable IN list WHERE condition | expression]` の形式でリストを変換・フィルタできます。

### 変換のみ

```cypher
-- 各要素を 2 倍
RETURN [x IN [1, 2, 3, 4, 5] | x * 2]
-- 結果: [2, 4, 6, 8, 10]

-- 文字列変換
RETURN [name IN ["alice", "bob", "charlie"] | toUpper(name)]
-- 結果: ["ALICE", "BOB", "CHARLIE"]
```

### フィルタのみ

```cypher
-- 偶数のみ
RETURN [x IN [1, 2, 3, 4, 5] WHERE x % 2 = 0]
-- 結果: [2, 4]
```

### フィルタと変換の組み合わせ

```cypher
-- 偶数を 2 倍
RETURN [x IN [1, 2, 3, 4, 5] WHERE x % 2 = 0 | x * 2]
-- 結果: [4, 8]
```

### ノードのプロパティを変換

```cypher
-- 友人の名前リストを取得
MATCH (p:Person {name: "Alice"})
RETURN [friend IN [(p)-[:KNOWS]->(f) | f] | friend.name] AS friend_names
```

## IN 演算子

値がリストに含まれるかを確認します。

```cypher
RETURN 3 IN [1, 2, 3, 4, 5]
-- 結果: true

RETURN 6 IN [1, 2, 3, 4, 5]
-- 結果: false

-- プロパティチェック
MATCH (n:Person)
WHERE n.city IN ["Tokyo", "Osaka", "Kyoto"]
RETURN n.name, n.city
```

## 実用的な例

```cypher
-- タグの共通部分を確認（手動実装）
MATCH (a:Article), (b:Article)
WHERE a.id <> b.id
WITH a, b,
     [tag IN a.tags WHERE tag IN b.tags] AS common_tags
WHERE size(common_tags) > 0
RETURN a.title, b.title, common_tags

-- ページネーション付きリスト処理
MATCH (n:Person)
WITH collect(n.name) AS all_names
RETURN all_names[($page - 1) * $size .. $page * $size] AS page_names
```

## ノード・グラフ関連のリスト関数

### labels(node)

ノードが持つラベルの一覧を文字列リストで返します。複数ラベルを持つノードでは全ラベルが含まれます。

```cypher
-- ノードのラベル一覧を取得
MATCH (n:Person {name: "Alice"})
RETURN labels(n)
-- 結果: ["Person", "Employee"]  （複数ラベルの場合）

-- ラベルを条件に使用
MATCH (n)
WHERE "Admin" IN labels(n)
RETURN n.name

-- ラベル数でフィルタ
MATCH (n)
WHERE size(labels(n)) > 1
RETURN n.name, labels(n)
```
