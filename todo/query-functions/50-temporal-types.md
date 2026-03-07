# 日時型（テンポラル型）

## 概要
Cypher の日時型（`date()`, `datetime()`, `duration()`）を実装する。
現在は `timestamp()`（Unix ミリ秒整数）のみ対応。

## 実装内容

### PropertyValue の拡張

- [x] `PropertyValue::Date(NaiveDate)` を追加
- [x] `PropertyValue::DateTime(DateTime<Utc>)` を追加
- [x] `PropertyValue::Duration(Duration)` を追加

### 組み込み関数

- [x] `date()` - 現在日付、または `date("2024-01-15")` でパース
- [ ] `date({year, month, day})` - マップから構築（Map式未サポートのため保留）
- [x] `datetime()` - 現在日時（UTC）
- [x] `datetime("2024-01-15T12:00:00Z")` - ISO 8601 文字列からパース
- [x] `duration("P1Y2M3D")` - ISO 8601 期間文字列からパース
- [ ] `duration({years, months, days, hours, minutes, seconds})` - マップから構築（同上）

### 日時演算

- [x] 日付 + duration: `date() + duration("P1D")`
- [x] 日時の差: `datetime2 - datetime1 = duration`
- [x] 比較演算子: `<`, `>`, `<=`, `>=` が日時・期間に対応

### アクセサ関数

- [x] `.year`, `.month`, `.day` プロパティアクセス
- [x] `.hour`, `.minute`, `.second`
- [x] `duration.days`, `duration.hours` 等

### 永続化対応

- [x] `PropertyValue::Date/DateTime/Duration` のシリアライズ・デシリアライズ

## クエリ例

```cypher
-- 現在日付でノード作成
CREATE (e:Event {name: "Conference", date: date()})

-- 特定日付でフィルタ
MATCH (e:Event)
WHERE e.date >= date("2024-01-01") AND e.date < date("2025-01-01")
RETURN e.name, e.date

-- 期間計算
MATCH (p:Person)
RETURN p.name, duration.between(p.birthdate, date()).years AS age

-- 日時の演算
RETURN datetime() + duration("PT1H") AS one_hour_later
```

## 依存クレート

- `chrono` - 日時型（すでに間接的に利用されている可能性あり）

## 依存

- `03-query-parser.md` が完了していること
- `09-persistence-format.md` が完了していること

## 対象クレート

`maharit-core`, `maharit-query`, `maharit-storage`
