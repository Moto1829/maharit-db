---
title: 全文検索インデックス
parent: インデックス・制約
nav_order: 2
---

# 全文検索インデックス

全文検索インデックスを使用することで、テキストデータに対する高精度な検索が可能になります。BM25 スコアリングと日本語形態素解析をサポートします。

## インデックスの作成

```cypher
-- 基本的な全文検索インデックスの作成
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title]

-- 複数プロパティのインデックス
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title, a.body, a.summary]

-- 複数ラベルのインデックス
CREATE FULLTEXT INDEX contentIndex FOR (n:Article|Post|Page) ON EACH [n.title, n.body]
```

### 構文

```
CREATE FULLTEXT INDEX <index_name>
FOR (<variable>:<Label>[|<Label>...])
ON EACH [<variable>.<property>[, <variable>.<property>...]]
```

## インデックスの削除

```cypher
DROP FULLTEXT INDEX articleIndex
```

## インデックスの一覧

```cypher
SHOW INDEXES
```

全文検索インデックスも一覧に含まれます。

## 全文検索の実行

`CALL db.index.fulltext.search()` を使用してインデックスを検索します。

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust プログラミング")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 10
```

### パラメータ

| パラメータ | 型 | 説明 |
|------------|-----|------|
| `indexName` | String | 検索するインデックスの名前 |
| `query` | String | 検索クエリ |

### 戻り値

| 列名 | 型 | 説明 |
|------|-----|------|
| `node` | Node | マッチしたノード |
| `score` | Float | BM25 スコア |

## 検索クエリの書き方

### 単一キーワード

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust")
YIELD node, score
RETURN node.title, score
```

### 複数キーワード（AND 検索）

スペース区切りで複数のキーワードを指定します。すべてのキーワードを含む文書が上位にランクされます。

```cypher
CALL db.index.fulltext.search("articleIndex", "グラフ データベース")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

### フレーズ検索

ダブルクォートで囲むことで、正確なフレーズを検索します。

```cypher
CALL db.index.fulltext.search("articleIndex", "\"グラフデータベース\"")
YIELD node, score
RETURN node.title, score
```

### ファジー検索

`~N` で編集距離を指定し、スペルゆれを許容します。

```cypher
CALL db.index.fulltext.search("articleIndex", "databse~1")
YIELD node, score
RETURN node.title, score
```

## 日本語対応

MaharitDB の全文検索は lindera（IPADIC 辞書）を使った日本語形態素解析に対応しています。

```cypher
-- 日本語テキストのインデックス
CREATE FULLTEXT INDEX jpIndex FOR (d:Document) ON EACH [d.content]

-- 形態素解析を通じた検索
-- 「機械学習」は「機械」「学習」に分割されて検索されます
CALL db.index.fulltext.search("jpIndex", "機械学習")
YIELD node, score
RETURN node.content, score
ORDER BY score DESC
```

## 追加情報との組み合わせ

全文検索の結果に対してさらにクエリを実行できます。

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust async")
YIELD node AS article, score
WHERE score > 1.0
MATCH (article)-[:AUTHORED_BY]->(author:Person)
MATCH (article)-[:HAS_TAG]->(tag:Tag)
RETURN article.title, author.name, collect(tag.name) AS tags, score
ORDER BY score DESC
LIMIT 5
```

## パフォーマンスに関する注意

- 全文検索インデックスはインデックス更新時に自動的に再構築されます
- 大量のドキュメントがある場合、インデックスの構築に時間がかかることがあります
- スコア閾値（`WHERE score > N`）を設定することで不要な結果を除外できます
- インデックスに含めるプロパティが多いほど、インデックスサイズが増加します
