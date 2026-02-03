# クエリ拡張: 高度なクエリ構文

## 概要
より複雑なクエリを記述するための構文を実装する。

## 実装内容

### WITH句
- [x] 中間結果の変数バインド
- [x] パイプライン処理
- [x] 集計結果の再利用

### OPTIONAL MATCH
- [x] マッチしない場合にNULLを返す
- [x] 複数OPTIONAL MATCHの組み合わせ

### UNION
- [ ] UNION（重複排除）
- [ ] UNION ALL（重複許容）
- [ ] スキーマ互換性チェック

### CASE WHEN
- [x] 単純CASE式
- [x] 検索CASE式
- [x] ELSE句

### 正規表現マッチ
- [ ] `=~` 演算子の実装
- [ ] 正規表現リテラルのパース

## クエリ例
```cypher
-- WITH句
MATCH (n:Person)
WITH n.city AS city, COUNT(*) AS count
WHERE count > 5
RETURN city, count

-- OPTIONAL MATCH
MATCH (a:Person)
OPTIONAL MATCH (a)-[:KNOWS]->(b)
RETURN a.name, b.name

-- UNION
MATCH (n:Person) RETURN n.name AS name
UNION
MATCH (n:Company) RETURN n.name AS name

-- CASE WHEN
MATCH (n:Person)
RETURN n.name,
  CASE
    WHEN n.age < 20 THEN 'young'
    WHEN n.age < 60 THEN 'adult'
    ELSE 'senior'
  END AS category

-- 正規表現
MATCH (n:Person) WHERE n.name =~ 'A.*' RETURN n
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
