# クエリ拡張: ソート・ページネーション

## 概要
クエリ結果のソートとページネーション機能を実装する。

## 実装内容

### ORDER BY
- [x] 単一カラムでのソート
- [x] 複数カラムでのソート
- [x] ASC/DESC指定
- [x] NULL値の扱い（NULLS FIRST/LAST）

### LIMIT / SKIP
- [x] LIMIT句の実装
- [x] SKIP句の実装
- [x] 組み合わせ: `SKIP 10 LIMIT 20`

### DISTINCT
- [x] RETURN DISTINCTの実装
- [x] 複数カラムでの重複排除

### パーサー拡張
- [x] ORDER BY句のパース
- [x] LIMIT/SKIP句のパース
- [x] DISTINCT修飾子のパース
- [x] NULLS FIRST/LAST句のパース

### 実行エンジン拡張
- [x] ソート処理の実装
- [x] メモリ効率の良いソート（TopN選択によるLIMIT最適化）
- [x] ページネーション処理

## クエリ例
```cypher
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC, n.name ASC
SKIP 10 LIMIT 20

MATCH (n:Person)
RETURN DISTINCT n.city

MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age ASC NULLS FIRST
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
