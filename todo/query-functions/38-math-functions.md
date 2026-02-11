# 数学関数

## 概要
Cypherの組み込み数学関数を実装する。

## 実装内容

### 基本数学関数
- [ ] abs(value): 絶対値
- [ ] ceil(value): 切り上げ
- [ ] floor(value): 切り捨て
- [ ] round(value): 四捨五入
- [ ] round(value, precision): 指定桁数で四捨五入
- [ ] sign(value): 符号（-1, 0, 1）

### ユーティリティ
- [ ] rand(): 0以上1未満のランダム数
- [ ] isNaN(value): NaNチェック

### 対数・指数（将来的）
- [ ] log(value): 自然対数
- [ ] log10(value): 常用対数
- [ ] sqrt(value): 平方根
- [ ] e(): ネイピア数
- [ ] pi(): 円周率

## クエリ例
```cypher
-- 絶対値
MATCH (n:Product) RETURN abs(n.price - 100) AS diff

-- 四捨五入
MATCH (n:Product) RETURN round(n.price, 2) AS rounded_price

-- 切り上げ・切り捨て
MATCH (n:Product) RETURN ceil(n.rating) AS ceil_rating, floor(n.rating) AS floor_rating

-- ランダム
MATCH (n:Person) RETURN n.name, rand() AS random_score ORDER BY random_score
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
