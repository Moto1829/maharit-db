# 全文検索インデックス

**Status**: Completed

## 概要
テキストプロパティに対する全文検索機能を実装する。

## 実装内容

### インデックス構造
- [x] 転置インデックスの実装
- [x] トークナイザー（空白区切り、N-gram）
- [x] 日本語対応（形態素解析 - lindera/sudachi）

### インデックス管理
- [x] 全文インデックスの作成
- [x] インデックスの削除
- [x] インデックスの更新（ノード変更時）

### 検索機能
- [x] キーワード検索
- [x] フレーズ検索
- [x] ブール検索（AND/OR/NOT）
- [x] ファジー検索（編集距離）
- [x] スコアリング（TF-IDF/BM25）

### クエリ構文
- [x] CONTAINS述語
- [x] SEARCH関数

## クエリ例
```cypher
-- 全文インデックス作成
CREATE FULLTEXT INDEX article_content FOR (n:Article) ON (n.title, n.body)

-- 検索
MATCH (n:Article)
WHERE n.title CONTAINS 'graph database'
RETURN n

-- スコア付き検索
CALL db.index.fulltext.search('article_content', 'graph database')
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

## 依存クレート候補
- `tantivy` - 全文検索エンジン
- `lindera` - 日本語形態素解析

## 依存
- `08-property-index.md` が完了していること

## 対象クレート
`maharit-core` または新規 `maharit-search`
