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

MaharitDB は [lindera](https://github.com/lindera/lindera) と埋め込み IPADIC 辞書を使用した日本語形態素解析に対応しています。日本語テキストは自動的に形態素（単語）に分割されてインデックス・検索されます。

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

### `japanese` feature の有効化

日本語形態素解析は **`japanese` という Cargo feature でゲートされており、既定では無効** です。埋め込み IPADIC 辞書（約 15MB）がバイナリサイズを大きく押し上げるため、日本語全文検索が必要なリリースでのみ有効化する方針になっています。

| ビルド | バイナリサイズ（実測） |
| --- | --- |
| 既定（`japanese` 無効） | 約 2.62MB |
| `japanese` 有効 | 約 18MB |

```bash
# 日本語形態素解析を含む本番バイナリをビルド
cargo build --release -p maharit-server --features japanese

# テストも feature を有効にして実行
cargo test -p maharit-core --features japanese
```

feature は `maharit-server` の `japanese` が `maharit-core/japanese` を引き込み、`maharit-core/japanese` がオプション依存 `lindera`（`embed-ipadic` feature 付き）を有効化する構成です。

> **feature が無効なビルドでの挙動**: 日本語テキストも一般のテキストと同じく「英数字以外の文字で分割」されます。日本語には単語境界の空白がないため、文全体が 1 トークンになりやすく、実用的な分かち書き検索はできません（CONTAINS による部分一致は引き続き機能します）。

### 形態素解析の仕様

`japanese` feature が有効な場合、トークナイザは入力テキストごとに次の判定・処理を行います。

1. **日本語判定**: テキストにひらがな（U+3040–U+309F）・カタカナ（U+30A0–U+30FF）・CJK 統合漢字（U+4E00–U+9FFF）のいずれかが 1 文字でも含まれていれば「日本語」とみなし、lindera 経由で形態素解析します。1 文字も含まれない純 ASCII テキストは、feature の有無にかかわらず従来の「英数字以外で分割 + 小文字化」のトークナイザを使います。
2. **辞書**: 埋め込み IPADIC 辞書（`embedded://ipadic`）をロードし、`Mode::Normal` で分割します。
3. **ストップ品詞の除外**: 次の品詞（IPADIC の主品詞）は検索ノイズになるためトークンから除外します。
   - 助詞 / 助動詞 / 記号 / 接続詞 / 感動詞 / フィラー / 非言語音
4. **原形（基本形）への正規化**: 各トークンは IPADIC の基本形（活用前の形）に変換されます。基本形が取得できない、または `*` の場合は表層形（surface form）にフォールバックします。最終的にすべて小文字化されます。

例: 「グラフは高速です」→ 助詞「は」と助動詞「です」が除外され、「グラフ」「高速」が残ります。

### 日英混在テキスト

テキストに日本語文字が 1 文字でも含まれると、文字列全体が日本語トークナイザ（lindera）にルーティングされます。そのため「Rust言語でグラフDBを実装」のような混在文では、英単語部分の分割は lindera のセグメンテーション結果に依存します（英単語が必ずトークンとして残るとは限りません）。純 ASCII の検索クエリ・文書のみ従来トークナイザを通ります。

### エラー時のフォールバック

辞書ロードや形態素解析が何らかの理由で失敗した場合でも、インデックス対象テキストを黙って破棄しないよう、空白および英数字以外の文字での分割にフォールバックします。

### 並列インデックス構築

大量ノードの一括インデックス（`build_index` / `build_index_bulk`）では、トークナイズ処理を rayon で並列化します（しきい値 200 ドキュメント以上）。各ワーカースレッドは lindera トークナイザをスレッドローカルにキャッシュするため、高コストな辞書ロードはスレッドごとに最大 1 回で済みます。

### 制限事項

- 形態素解析の精度は IPADIC 辞書の語彙に依存します。新語・固有名詞・専門用語は意図しない分割になることがあります（ユーザー辞書には未対応）。
- 利用できる辞書は埋め込み IPADIC のみで、NEologd など他辞書への切り替えには対応していません。
- 日本語判定は文字種ベースのため、全角英数字のみの文字列など一部のケースでは日本語トークナイザに回らないことがあります。

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
