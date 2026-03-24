---
title: 全文検索関数
parent: 関数リファレンス
nav_order: 6
---

# 全文検索関数

MaharitDB は BM25 アルゴリズムに基づく全文検索エンジンを内蔵しています。日本語形態素解析（lindera IPADIC）にも対応しています。

## 全文検索インデックスの作成

全文検索を使用する前に、対象のノードとプロパティに対してインデックスを作成する必要があります。

```cypher
-- 単一プロパティのインデックス
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title]

-- 複数プロパティのインデックス
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title, a.body, a.summary]

-- 複数ラベルのインデックス
CREATE FULLTEXT INDEX contentIndex FOR (n:Article|Post|Page) ON EACH [n.title, n.body]
```

## CALL db.index.fulltext.search()

インデックスを使った全文検索を実行します。

```cypher
-- 基本的な検索
CALL db.index.fulltext.search("articleIndex", "Rust programming")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 10
```

戻り値:
- `node`: マッチしたノード
- `score`: BM25 スコア（高いほど関連性が高い）

## 検索クエリの構文

### キーワード検索

複数のキーワードを含む文書を検索します（AND 検索）。

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

### ファジー検索（編集距離指定）

チルダ（`~`）に続けて編集距離を指定することで、スペルゆれを許容した検索ができます。

```cypher
-- 編集距離 1 以内のゆらぎを許容
CALL db.index.fulltext.search("articleIndex", "graph~1")
YIELD node, score
RETURN node.title, score

-- 編集距離 2 以内
CALL db.index.fulltext.search("articleIndex", "datbase~2")
YIELD node, score
RETURN node.title, score
```

### OR 検索

`OR` キーワードを使用して、いずれかのキーワードを含む文書を検索します。

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust OR Go OR Python")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

## CONTAINS 述語

全文検索インデックスがある場合、`CONTAINS` をフィルタとして使えます。

```cypher
MATCH (a:Article)
WHERE a.body CONTAINS "グラフ"
RETURN a.title
```

注意: `CONTAINS` は部分一致の文字列検索です。BM25 スコアは得られません。高精度な検索には `CALL db.index.fulltext.search()` を使用してください。

## 日本語全文検索

lindera IPADIC を使用した日本語形態素解析に対応しています。日本語テキストは自動的に形態素に分割されて検索されます。

```cypher
-- 日本語テキストの全文検索
CREATE FULLTEXT INDEX jpArticleIndex FOR (a:Article) ON EACH [a.title, a.body]

-- 形態素に基づいた検索（「グラフデータベース」→「グラフ」「データベース」で検索）
CALL db.index.fulltext.search("jpArticleIndex", "グラフデータベース")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC

-- 複数のキーワードで絞り込み
CALL db.index.fulltext.search("jpArticleIndex", "機械学習 グラフ")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 5
```

## BM25 スコアリングの詳細

MaharitDB の BM25 実装は次の式を使用します：

```
score(q, d) = Σ IDF(qi) * (f(qi,d) * (k1+1)) / (f(qi,d) + k1 * (1 - b + b * |d| / avgdl))
```

パラメータ:
- `k1 = 1.2`（単語の出現頻度の影響係数）
- `b = 0.75`（文書の長さによる正規化係数）
- IDF: `ln(1 + (N - df + 0.5) / (df + 0.5))`（負のスコアを避けるための variant）

## インデックスの管理

```cypher
-- インデックスの削除
DROP FULLTEXT INDEX articleIndex

-- 全インデックスの一覧（プロパティインデックスを参照）
SHOW INDEXES
```

## 実用的な例

```cypher
-- 関連記事の検索（スコア閾値でフィルタ）
CALL db.index.fulltext.search("articleIndex", $query)
YIELD node, score
WHERE score > 0.5
RETURN node.title, node.url, score
ORDER BY score DESC
LIMIT 5

-- 検索結果に追加情報を付加
CALL db.index.fulltext.search("articleIndex", "Rust async await")
YIELD node, score
MATCH (node)-[:HAS_TAG]->(tag:Tag)
RETURN node.title, score, collect(tag.name) AS tags
ORDER BY score DESC
```
