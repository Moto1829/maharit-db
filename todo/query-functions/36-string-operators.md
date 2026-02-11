# 文字列演算子

## 概要
Cypherの文字列比較演算子を実装する。

## 実装内容

### 文字列比較演算子
- [ ] STARTS WITH: 前方一致
- [ ] ENDS WITH: 後方一致
- [x] CONTAINS: 部分一致（実装済み）
- [ ] IS NORMALIZED: Unicode正規化チェック

## クエリ例
```cypher
-- 前方一致
MATCH (n:Person) WHERE n.name STARTS WITH 'A' RETURN n

-- 後方一致
MATCH (n:Person) WHERE n.name ENDS WITH 'son' RETURN n

-- 部分一致（実装済み）
MATCH (n:Person) WHERE n.name CONTAINS 'ali' RETURN n
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
