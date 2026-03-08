---
title: 数学関数
parent: 関数リファレンス
nav_order: 2
---

# 数学関数

MaharitDB は Cypher クエリ内で使用できる基本的な数学関数を提供します。

## 絶対値・符号

### abs(number)

数値の絶対値を返します。

```cypher
RETURN abs(-42)
-- 結果: 42

RETURN abs(3.14)
-- 結果: 3.14
```

### sign(number)

数値の符号を返します。正の場合は `1`、負の場合は `-1`、0 の場合は `0`。

```cypher
RETURN sign(-5)
-- 結果: -1

RETURN sign(0)
-- 結果: 0

RETURN sign(3.14)
-- 結果: 1
```

## 丸め

### ceil(number)

数値を切り上げた整数を返します。

```cypher
RETURN ceil(3.1)
-- 結果: 4.0

RETURN ceil(-3.1)
-- 結果: -3.0
```

### floor(number)

数値を切り下げた整数を返します。

```cypher
RETURN floor(3.9)
-- 結果: 3.0

RETURN floor(-3.1)
-- 結果: -4.0
```

### round(number)

数値を四捨五入した整数を返します。

```cypher
RETURN round(3.5)
-- 結果: 4.0

RETURN round(3.4)
-- 結果: 3.0

RETURN round(-3.5)
-- 結果: -4.0
```

## べき乗・平方根

### sqrt(number)

数値の平方根を返します。

```cypher
RETURN sqrt(16.0)
-- 結果: 4.0

RETURN sqrt(2.0)
-- 結果: 1.4142135623730951
```

### pow(base, exponent)

`base` の `exponent` 乗を返します。

```cypher
RETURN pow(2, 10)
-- 結果: 1024.0

RETURN pow(3.0, 0.5)
-- 結果: 1.7320508075688772
```

## 対数・指数

### log(number)

自然対数（底 e）を返します。

```cypher
RETURN log(2.718281828)
-- 結果: 約 1.0
```

### log10(number)

常用対数（底 10）を返します。

```cypher
RETURN log10(1000)
-- 結果: 3.0
```

### exp(number)

e の `number` 乗を返します。

```cypher
RETURN exp(1)
-- 結果: 2.718281828459045
```

## 三角関数

```cypher
-- sin, cos, tan
RETURN sin(0)    -- 結果: 0.0
RETURN cos(0)    -- 結果: 1.0
RETURN tan(0)    -- 結果: 0.0

-- asin, acos, atan
RETURN asin(1.0)  -- 結果: 1.5707963267948966 (π/2)
RETURN acos(1.0)  -- 結果: 0.0
RETURN atan(1.0)  -- 結果: 0.7853981633974483 (π/4)

-- atan2(y, x)
RETURN atan2(1.0, 1.0)  -- 結果: 0.7853981633974483
```

## 定数

```cypher
-- 円周率 π
RETURN pi()
-- 結果: 3.141592653589793

-- e（ネイピア数）
RETURN e()
-- 結果: 2.718281828459045
```

## 乱数

### rand()

0.0 以上 1.0 未満の一様乱数を返します。

```cypher
RETURN rand()
-- 結果: 例 0.7234567890...

-- ランダムなサンプリング（約 10% のノードを返す）
MATCH (n:Person)
WHERE rand() < 0.1
RETURN n.name
```

## 型変換

### toInteger(number)

浮動小数点数を整数に変換します（小数部分を切り捨て）。

```cypher
RETURN toInteger(3.9)
-- 結果: 3

RETURN toInteger(-3.1)
-- 結果: -3
```

### toFloat(integer)

整数を浮動小数点数に変換します。

```cypher
RETURN toFloat(42)
-- 結果: 42.0
```

## 実用的な例

```cypher
-- 距離計算（ピタゴラスの定理）
MATCH (a:Location), (b:Location)
WHERE a.name = "A" AND b.name = "B"
RETURN sqrt(pow(b.x - a.x, 2) + pow(b.y - a.y, 2)) AS distance

-- 価格を 10% 引き上げて切り上げ
MATCH (p:Product)
SET p.new_price = ceil(p.price * 1.1)

-- 統計的なサンプリング
MATCH (n:Event)
WHERE rand() < $sample_rate
RETURN n.id, n.timestamp
LIMIT 1000
```
