# 文字列関数

MaharitDB は Cypher クエリ内で使用できる豊富な文字列関数を提供します。

## 大文字・小文字変換

### toLower(string)

文字列をすべて小文字に変換します。

```cypher
RETURN toLower("Hello World")
-- 結果: "hello world"

MATCH (n:Person)
WHERE toLower(n.name) = "alice"
RETURN n
```

### toUpper(string)

文字列をすべて大文字に変換します。

```cypher
RETURN toUpper("Hello World")
-- 結果: "HELLO WORLD"
```

## 空白処理

### trim(string)

文字列の先頭と末尾の空白を除去します。

```cypher
RETURN trim("  hello  ")
-- 結果: "hello"
```

### ltrim(string)

文字列の先頭の空白を除去します。

```cypher
RETURN ltrim("  hello  ")
-- 結果: "hello  "
```

### rtrim(string)

文字列の末尾の空白を除去します。

```cypher
RETURN rtrim("  hello  ")
-- 結果: "  hello"
```

## 部分文字列

### substring(string, start[, length])

文字列の部分を取得します。`start` は 0 始まりのインデックスです。

```cypher
RETURN substring("Hello World", 6)
-- 結果: "World"

RETURN substring("Hello World", 0, 5)
-- 結果: "Hello"
```

### left(string, count)

文字列の左から `count` 文字を取得します。

```cypher
RETURN left("Hello World", 5)
-- 結果: "Hello"
```

### right(string, count)

文字列の右から `count` 文字を取得します。

```cypher
RETURN right("Hello World", 5)
-- 結果: "World"
```

## 文字列情報

### size(string)

文字列の文字数を返します。

```cypher
RETURN size("Hello")
-- 結果: 5

MATCH (n:Person)
WHERE size(n.name) > 5
RETURN n.name
```

### length(string)

`size` と同様に文字列の長さを返します。

```cypher
RETURN length("Hello")
-- 結果: 5
```

## 検索・一致

### startsWith(string, prefix)

文字列が指定のプレフィックスで始まるかを返します。

```cypher
RETURN startsWith("Hello World", "Hello")
-- 結果: true

MATCH (n:Person)
WHERE n.name STARTS WITH "Al"
RETURN n.name
```

### endsWith(string, suffix)

文字列が指定のサフィックスで終わるかを返します。

```cypher
RETURN endsWith("alice@example.com", "@example.com")
-- 結果: true
```

### contains(string, substring)

文字列に指定の部分文字列が含まれるかを返します。

```cypher
RETURN contains("Hello World", "World")
-- 結果: true

MATCH (n:Article)
WHERE n.body CONTAINS "グラフ"
RETURN n.title
```

## 分割・結合

### split(string, delimiter)

文字列を区切り文字で分割し、リストを返します。

```cypher
RETURN split("a,b,c", ",")
-- 結果: ["a", "b", "c"]

-- タグ文字列を展開
MATCH (a:Article)
UNWIND split(a.tag_string, ",") AS tag
RETURN a.title, trim(tag) AS tag
```

### replace(string, search, replacement)

文字列内の検索文字列をすべて置換します。

```cypher
RETURN replace("Hello World", "World", "Rust")
-- 結果: "Hello Rust"
```

## 型変換

### toString(value)

数値や真偽値を文字列に変換します。

```cypher
RETURN toString(42)
-- 結果: "42"

RETURN toString(3.14)
-- 結果: "3.14"

RETURN toString(true)
-- 結果: "true"
```

### toInteger(string)

文字列を整数に変換します。変換できない場合は `null` を返します。

```cypher
RETURN toInteger("42")
-- 結果: 42

RETURN toInteger("abc")
-- 結果: null
```

### toFloat(string)

文字列を浮動小数点数に変換します。

```cypher
RETURN toFloat("3.14")
-- 結果: 3.14
```

## 実用的な例

```cypher
-- メールアドレスのドメインを抽出
MATCH (u:User)
RETURN u.email,
       substring(u.email, size(split(u.email, "@")[0]) + 1) AS domain

-- 名前を正規化して一致チェック
MATCH (n:Person)
WHERE toLower(trim(n.name)) = toLower(trim($search_name))
RETURN n

-- ユーザー名を生成
MATCH (p:Person)
RETURN toLower(replace(p.name, " ", "_")) AS username
```
