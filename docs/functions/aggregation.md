---
title: 集計関数
parent: 関数リファレンス
nav_order: 5
---

# 集計関数

集計関数は複数の行をまとめて単一の値（または結果）を計算します。`RETURN` または `WITH` 句で使用します。

## 基本的な集計関数

### COUNT

行数またはユニークな値の数を返します。

```cypher
-- 全ノード数
MATCH (n:Person) RETURN count(n)

-- プロパティが存在する行数（null を除外）
MATCH (n:Person) RETURN count(n.email)

-- ユニークな値の数
MATCH (n:Person) RETURN count(DISTINCT n.city)
```

### SUM

数値の合計を返します。

```cypher
MATCH (p:Product)
RETURN sum(p.price) AS total_price

-- DISTINCT（重複を排除して合計）
MATCH (o:Order)-[:INCLUDES]->(p:Product)
RETURN sum(DISTINCT p.price) AS unique_product_total
```

### AVG

数値の平均を返します。

```cypher
MATCH (n:Person)
RETURN avg(n.age) AS average_age

MATCH (p:Product)
RETURN p.category, avg(p.price) AS avg_price
ORDER BY avg_price DESC
```

### MAX / MIN

最大値・最小値を返します。

```cypher
MATCH (n:Person)
RETURN max(n.age) AS oldest, min(n.age) AS youngest

-- 文字列の MAX/MIN（辞書順）
MATCH (n:Person)
RETURN max(n.name), min(n.name)
```

### COLLECT

値をリストに収集します。

```cypher
-- 全員の名前をリストにまとめる
MATCH (n:Person)
RETURN collect(n.name) AS names

-- 友人の名前を収集
MATCH (p:Person)-[:KNOWS]->(f:Person)
RETURN p.name, collect(f.name) AS friends

-- DISTINCT（重複を排除して収集）
MATCH (n:Person)
RETURN collect(DISTINCT n.city) AS unique_cities
```

## パーセンタイル

### percentileCont(value, percentile)

線形補間を使った連続パーセンタイルを計算します。`percentile` は 0.0 から 1.0 の範囲で指定します。

```cypher
MATCH (n:Person)
RETURN percentileCont(n.age, 0.5) AS median_age

-- 第 75 パーセンタイル
MATCH (p:Product)
RETURN percentileCont(p.price, 0.75) AS p75_price
```

### percentileDisc(value, percentile)

離散パーセンタイルを計算します（実際の値から選択）。

```cypher
MATCH (n:Person)
RETURN percentileDisc(n.age, 0.5) AS median_age

MATCH (p:Product)
RETURN percentileDisc(p.price, 0.90) AS p90_price
```

## 標準偏差

### stDev(value)

標本標準偏差（n-1 で除算）を計算します。

```cypher
MATCH (n:Person)
RETURN stDev(n.age) AS age_std_dev

-- カテゴリ別の価格ばらつき
MATCH (p:Product)
RETURN p.category, stDev(p.price) AS price_deviation
ORDER BY price_deviation DESC
```

### stDevP(value)

母標準偏差（n で除算）を計算します。

```cypher
MATCH (n:Person)
RETURN stDevP(n.age) AS age_std_dev_population
```

## グループ化

集計関数を使用すると、非集計の列でグループ化が自動的に行われます。

```cypher
-- 都市別の人数と平均年齢
MATCH (n:Person)
RETURN n.city, count(n) AS count, avg(n.age) AS avg_age
ORDER BY count DESC

-- カテゴリ別の商品統計
MATCH (p:Product)
RETURN
  p.category,
  count(p) AS product_count,
  min(p.price) AS min_price,
  max(p.price) AS max_price,
  avg(p.price) AS avg_price,
  sum(p.stock) AS total_stock
ORDER BY product_count DESC
```

## WITH との組み合わせ

集計結果をさらにフィルタリングするには `WITH` を使います。

```cypher
-- 友人が 3 人以上いる人
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, count(f) AS friend_count
WHERE friend_count >= 3
RETURN p.name, friend_count
ORDER BY friend_count DESC

-- 売上が平均以上のカテゴリ
MATCH (p:Product)
WITH p.category AS category, sum(p.sales) AS total_sales
WITH collect({category: category, sales: total_sales}) AS stats,
     avg(total_sales) AS avg_sales
UNWIND stats AS stat
WHERE stat.sales >= avg_sales
RETURN stat.category, stat.sales
ORDER BY stat.sales DESC
```

## 実用的な例

```cypher
-- ユーザーのアクティビティ統計
MATCH (u:User)-[:POSTED]->(p:Post)
RETURN
  u.name,
  count(p) AS post_count,
  max(p.created_at) AS last_post,
  avg(p.likes) AS avg_likes,
  sum(p.likes) AS total_likes
ORDER BY post_count DESC
LIMIT 10
```
