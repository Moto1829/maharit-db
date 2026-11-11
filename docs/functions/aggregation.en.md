---
title: Aggregate functions
parent: Function reference
nav_order: 5
source_commit: 6cd2966b
---

# Aggregate functions

Aggregate functions combine many rows into a single value (or result). Use them in `RETURN` or `WITH`.

## Basic aggregate functions

### COUNT

Returns the number of rows or of distinct values.

```cypher
// Number of nodes
MATCH (n:Person) RETURN count(n)

// Number of rows where the property exists (null is excluded)
MATCH (n:Person) RETURN count(n.email)

// Number of distinct values
MATCH (n:Person) RETURN count(DISTINCT n.city)
```

### SUM

Returns the sum of numbers.

```cypher
MATCH (p:Product)
RETURN sum(p.price) AS total_price

// DISTINCT (sum without duplicates)
MATCH (o:Order)-[:INCLUDES]->(p:Product)
RETURN sum(DISTINCT p.price) AS unique_product_total
```

### AVG

Returns the average of numbers.

```cypher
MATCH (n:Person)
RETURN avg(n.age) AS average_age

MATCH (p:Product)
RETURN p.category, avg(p.price) AS avg_price
ORDER BY avg_price DESC
```

### MAX / MIN

Returns the maximum or minimum value.

```cypher
MATCH (n:Person)
RETURN max(n.age) AS oldest, min(n.age) AS youngest

// MAX/MIN of strings (lexicographic order)
MATCH (n:Person)
RETURN max(n.name), min(n.name)
```

### COLLECT

Collects values into a list.

```cypher
// Collect everyone's name into a list
MATCH (n:Person)
RETURN collect(n.name) AS names

// Collect friends' names
MATCH (p:Person)-[:KNOWS]->(f:Person)
RETURN p.name, collect(f.name) AS friends

// DISTINCT (collect without duplicates)
MATCH (n:Person)
RETURN collect(DISTINCT n.city) AS unique_cities
```

## Percentiles

### percentileCont(value, percentile)

Computes a continuous percentile using linear interpolation. `percentile` is between 0.0 and 1.0.

```cypher
MATCH (n:Person)
RETURN percentileCont(n.age, 0.5) AS median_age

// 75th percentile
MATCH (p:Product)
RETURN percentileCont(p.price, 0.75) AS p75_price
```

### percentileDisc(value, percentile)

Computes a discrete percentile (picks an actual value).

```cypher
MATCH (n:Person)
RETURN percentileDisc(n.age, 0.5) AS median_age

MATCH (p:Product)
RETURN percentileDisc(p.price, 0.90) AS p90_price
```

## Standard deviation

### stDev(value)

Computes the sample standard deviation (divided by n-1).

```cypher
MATCH (n:Person)
RETURN stDev(n.age) AS age_std_dev

// Price spread by category
MATCH (p:Product)
RETURN p.category, stDev(p.price) AS price_deviation
ORDER BY price_deviation DESC
```

### stDevP(value)

Computes the population standard deviation (divided by n).

```cypher
MATCH (n:Person)
RETURN stDevP(n.age) AS age_std_dev_population
```

## Grouping

When aggregate functions are used, the non-aggregated columns automatically become the grouping keys.

```cypher
// Number of people and average age by city
MATCH (n:Person)
RETURN n.city, count(n) AS count, avg(n.age) AS avg_age
ORDER BY count DESC

// Product statistics by category
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

## With WITH

Use `WITH` to filter on aggregated results.

```cypher
// People with three or more friends
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, count(f) AS friend_count
WHERE friend_count >= 3
RETURN p.name, friend_count
ORDER BY friend_count DESC

// Categories whose sales are at or above the average
MATCH (p:Product)
WITH p.category AS category, sum(p.sales) AS total_sales
WITH collect({category: category, sales: total_sales}) AS stats,
     avg(total_sales) AS avg_sales
UNWIND stats AS stat
WITH stat, avg_sales
WHERE stat.sales >= avg_sales
RETURN stat.category, stat.sales
ORDER BY stat.sales DESC
```

## Practical examples

```cypher
// User activity statistics
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
