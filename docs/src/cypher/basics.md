# Cypher 基本構文

MaharitDB は Neo4j の Cypher に近い構文のクエリ言語をサポートしています。このページでは最も基本的なクエリパターンを説明します。

## ノードのパターン

ノードは丸括弧 `()` で表します。

```cypher
-- ラベルなしのノード
()

-- ラベル付きのノード
(:Person)

-- 変数束縛
(n:Person)

-- プロパティフィルタ
(n:Person {name: "Alice"})

-- 複数ラベル
(n:Person:Employee)
```

### 複数ラベル

ノードには複数のラベルを付与できます。ラベルはノードが属するカテゴリを表し、複数のカテゴリに同時に属することが可能です。

```cypher
-- 複数ラベルを持つノードを作成
CREATE (n:Person:Employee {name: "Alice", department: "Engineering"})

-- 複数ラベルでマッチ（AND 条件 — 両方のラベルを持つノードのみ）
MATCH (n:Person:Employee) RETURN n.name

-- 片方のラベルだけでマッチ
MATCH (n:Employee) RETURN n.name
```

MATCH でラベルを複数指定した場合、**すべてのラベルを持つノード**のみが返ります（OR ではなく AND）。

```cypher
-- ラベルの一覧を取得
MATCH (n:Person) RETURN n.name, labels(n)
-- 結果: ["Person", "Employee"] のようなリスト
```

## エッジ（リレーションシップ）のパターン

エッジは角括弧 `[]` と矢印で表します。

```cypher
-- 向き付きエッジ
(a)-[:KNOWS]->(b)

-- 逆向き
(a)<-[:KNOWS]-(b)

-- 向きなし（マッチ時のみ）
(a)-[:KNOWS]-(b)

-- 変数束縛とプロパティ
(a)-[r:KNOWS {since: 2020}]->(b)

-- ホップ数指定（可変長）
(a)-[:KNOWS*1..3]->(b)
```

## CREATE

ノードとエッジを作成します。

```cypher
-- ノードを作成
CREATE (n:Person {name: "Alice", age: 30})

-- エッジを作成（既存ノードを参照）
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS {since: 2021}]->(b)

-- 複数ノードを一度に作成
CREATE
  (alice:Person {name: "Alice"}),
  (bob:Person {name: "Bob"}),
  (alice)-[:FRIENDS]->(bob)
```

## MATCH

グラフからパターンに一致するデータを検索します。

```cypher
-- すべてのノードを取得
MATCH (n) RETURN n

-- 特定ラベルのノードを取得
MATCH (n:Person) RETURN n

-- プロパティでフィルタ
MATCH (n:Person {name: "Alice"}) RETURN n

-- エッジを含むパターン
MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b

-- 可変長パス
MATCH (a:Person)-[:KNOWS*1..3]->(b:Person)
WHERE a.name = "Alice"
RETURN b.name
```

## RETURN

クエリの結果として返す値を指定します。

```cypher
-- ノード全体を返す
MATCH (n:Person) RETURN n

-- プロパティを返す
MATCH (n:Person) RETURN n.name, n.age

-- エイリアスを付ける
MATCH (n:Person) RETURN n.name AS name, n.age AS age

-- 計算式を返す
MATCH (n:Person) RETURN n.name, n.age * 2 AS double_age

-- 定数を返す
RETURN 42, "hello", true

-- DISTINCT（重複排除）
MATCH (n:Person) RETURN DISTINCT n.name
```

## DELETE と DETACH DELETE

ノードやエッジを削除します。

```cypher
-- エッジを削除
MATCH (a:Person)-[r:KNOWS]->(b:Person)
WHERE a.name = "Alice" AND b.name = "Bob"
DELETE r

-- ノードを削除（エッジが存在しない場合のみ）
MATCH (n:Person {name: "Charlie"})
DELETE n

-- ノードと接続するエッジをすべて削除
MATCH (n:Person {name: "Dave"})
DETACH DELETE n
```

## コメント

`--` の後はコメントとして扱われます。

```cypher
-- これはコメントです
MATCH (n:Person) -- インラインコメントも使えます
RETURN n.name
```

## 複数ステートメント

セミコロン（`;`）でステートメントを区切ることができます。

```cypher
CREATE (a:Person {name: "Alice"});
CREATE (b:Person {name: "Bob"});
MATCH (a:Person), (b:Person)
WHERE a.name <> b.name
CREATE (a)-[:KNOWS]->(b);
```

## リテラル値

| 型 | 例 |
|---|---|
| 整数 | `42`, `-10`, `0` |
| 浮動小数点 | `3.14`, `-0.5`, `1.0e3` |
| 文字列 | `"hello"`, `"日本語"` |
| 真偽値 | `true`, `false` |
| null | `null` |
| リスト | `[1, 2, 3]`, `["a", "b"]` |
| マップ | `{name: "Alice", age: 30}` |
