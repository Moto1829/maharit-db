# Task 42: 集計関数拡張

## 概要
Cypherの追加集計関数とDISTINCT修飾子を実装する。

## 実装内容

### パーセンタイル
- [x] percentileCont(expr, percentile): 連続パーセンタイル（補間あり）
- [x] percentileDisc(expr, percentile): 離散パーセンタイル（最近値）

### 標準偏差
- [x] stDev(expr): 標本標準偏差
- [x] stDevP(expr): 母標準偏差

### DISTINCT修飾子
- [x] COUNT(DISTINCT expr)
- [x] COLLECT(DISTINCT expr)
- [x] SUM(DISTINCT expr)
- [x] AVG(DISTINCT expr)

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

## 実装メモ（2026-02-25）

### 変更ファイル
- `crates/maharit-query/src/ast.rs`: `AggregateFunction` enumに8バリアント追加
- `crates/maharit-query/src/parser.rs`: percentileCont/percentileDisc/stDev/stDevP のパース追加、COUNT/SUM/AVG/COLLECT にDISTINCT修飾子サポート追加
- `crates/maharit-query/src/executor.rs`: `aggregate_to_name`, `return_item_to_column_name`, `evaluate_aggregate` に新バリアント対応追加、8テスト追加

### 設計上の決定
- `percentileCont/percentileDisc` の percentile引数は `parse_expression()` で解析し `ReturnItem::Expr` にラップ（浮動小数点リテラルを `parse_return_item()` では解析できないため）
- `percentileDisc` は整数値を返す場合は `Value::Int`、小数値は `Value::Float` で返す
- `stDev` (標本標準偏差) の除数は `n-1`、`stDevP` (母標準偏差) の除数は `n`
- `CollectDistinct` は `Value::List` を返す（既存の `Collect` が `Value::String` を返すのと異なる）
- DISTINCT系の重複排除はキーを `format!("{}", val)` で文字列化して `HashSet` で管理

### テスト結果
- 全310テスト通過（新規8テスト追加）
