# 集計関数拡張

## 概要
Cypherの追加集計関数とDISTINCT修飾子を実装する。

## 実装内容

### パーセンタイル
- [ ] percentileCont(expr, percentile): 連続パーセンタイル（補間あり）
- [ ] percentileDisc(expr, percentile): 離散パーセンタイル（最近値）

### 標準偏差
- [ ] stDev(expr): 標本標準偏差
- [ ] stDevP(expr): 母標準偏差

### DISTINCT修飾子
- [ ] COUNT(DISTINCT expr)
- [ ] COLLECT(DISTINCT expr)
- [ ] SUM(DISTINCT expr)
- [ ] AVG(DISTINCT expr)

## クエリ例
```cypher
-- パーセンタイル
MATCH (n:Person) RETURN percentileCont(n.age, 0.5) AS median_age

-- 標準偏差
MATCH (n:Person) RETURN stDev(n.age) AS age_stddev

-- DISTINCT集計
MATCH (n:Person) RETURN COUNT(DISTINCT n.city) AS unique_cities

-- COLLECT DISTINCT
MATCH (n:Person) RETURN COLLECT(DISTINCT n.city) AS unique_city_list
```

## 依存
- `04-query-executor.md` が完了していること
- 基本集計関数（COUNT, SUM, AVG, MIN, MAX, COLLECT）が実装済み

## 対象クレート
`maharit-query`
