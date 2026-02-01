# クエリ拡張: ソート・ページネーション

## 概要
クエリ結果のソートとページネーション機能を実装する。

## 実装内容

### ORDER BY
- [ ] 単一カラムでのソート
- [ ] 複数カラムでのソート
- [ ] ASC/DESC指定
- [ ] NULL値の扱い（NULLS FIRST/LAST）

### LIMIT / SKIP
- [ ] LIMIT句の実装
- [ ] SKIP句の実装
- [ ] 組み合わせ: `SKIP 10 LIMIT 20`

### DISTINCT
- [ ] RETURN DISTINCTの実装
- [ ] 複数カラムでの重複排除

### パーサー拡張
- [ ] ORDER BY句のパース
- [ ] LIMIT/SKIP句のパース
- [ ] DISTINCT修飾子のパース

### 実行エンジン拡張
- [ ] ソート処理の実装
- [ ] メモリ効率の良いソート（大量データ対応）
- [ ] ページネーション処理

## クエリ例
```cypher
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC, n.name ASC
SKIP 10 LIMIT 20

MATCH (n:Person)
RETURN DISTINCT n.city
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
