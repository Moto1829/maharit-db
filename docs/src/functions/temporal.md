# テンポラル型関数

MaharitDB は Cypher の日時型（`date`、`datetime`、`duration`）をサポートします。

## 日付型 (`date`)

`date` は日付（年・月・日）を表します。内部的には 1970-01-01 からの日数として保存されます。

### 作成

```cypher
-- 現在日付
RETURN date()

-- 文字列からパース (ISO 8601 形式: YYYY-MM-DD)
RETURN date("2024-01-15")
```

### プロパティへの保存

```cypher
CREATE (e:Event {name: "Conference", date: date("2024-06-15")})
```

## 日時型 (`datetime`)

`datetime` は日時（年・月・日・時・分・秒・ミリ秒）を表します。内部的には Unix エポックからのミリ秒として保存されます。

### 作成

```cypher
-- 現在日時 (UTC)
RETURN datetime()

-- 文字列からパース (ISO 8601 形式)
RETURN datetime("2024-06-15T12:30:00Z")
RETURN datetime("2024-06-15T12:30:00.500Z")
```

## 期間型 (`duration`)

`duration` は時間の長さを表します。ISO 8601 期間形式でパースします。

### 作成

```cypher
-- ISO 8601 期間文字列
-- P[nY][nM][nD][T[nH][nM][nS]]
RETURN duration("P1Y")        -- 1年
RETURN duration("P1Y2M3D")    -- 1年2ヶ月3日
RETURN duration("PT2H30M")    -- 2時間30分
RETURN duration("P1DT12H")    -- 1日12時間
```

## 演算

### 日付の加算・減算

```cypher
-- date + duration → date
RETURN date("2024-01-01") + duration("P1D")  -- 2024-01-02
RETURN date("2024-01-01") + duration("P1M")  -- 2024-02-01

-- datetime + duration → datetime
RETURN datetime("2024-01-01T00:00:00Z") + duration("PT1H")

-- date - date → duration
RETURN date("2024-01-10") - date("2024-01-01")  -- P0Y0M9D

-- datetime - datetime → duration
RETURN datetime("2024-01-02T00:00:00Z") - datetime("2024-01-01T00:00:00Z")
```

## 比較

テンポラル型は `<`、`>`、`<=`、`>=`、`=` で比較できます。

```cypher
MATCH (e:Event)
WHERE e.date >= date("2024-01-01") AND e.date < date("2025-01-01")
RETURN e.name, e.date

MATCH (e:Event)
WHERE e.date = date("2024-06-15")
RETURN e.name
```

## 表示形式

| 型 | 表示例 |
|---|---|
| `Date` | `2024-01-15` |
| `DateTime` | `2024-01-15T12:30:00.000Z` |
| `Duration` | `P1Y2M3DT4H30M` |

## 永続化

テンポラル型はバイナリ形式（type byte 5/6/7）で永続化されます。バックアップ・リストアでも保持されます。
