# タスク: WITH + グループ集計で COUNT が null になる

## 概要

`WITH n.city AS city, COUNT(n) AS cnt RETURN city, cnt` のような WITH 句内でのグループ集計クエリにおいて、集計値（cnt）が正しく計算されず `null` が返る。また city が重複して返されることからグルーピングが機能していない。

## 失敗したテスト
- スクリプト: `scripts/query_feature_test.py`
- テスト: `WITH（パイプライン）` セクション — "Tokyo の cnt が 2（WITH グループ集計）"
- エラーメッセージ:
```
tokyo_row={'cnt': 'null', 'city': '"Tokyo"'} (cnt=null; null の場合は WITH 集計バグ: todo/bug 参照)
```

## 再現クエリ

```cypher
CREATE (:Person {name: 'Alice', city: 'Tokyo'})
CREATE (:Person {name: 'Charlie', city: 'Tokyo'})
CREATE (:Person {name: 'Bob', city: 'Osaka'})

MATCH (n:Person)
WITH n.city AS city, COUNT(n) AS cnt
RETURN city, cnt ORDER BY cnt DESC
```

期待結果:
```
city     cnt
Tokyo    2
Osaka    1
```

実際の結果:
```
city     cnt
Tokyo    null
Osaka    null
Tokyo    null
```

## 根本原因の分析

WITH 句でグループキーと集計関数を同時に使用する場合（GROUP BY 相当）、executor.rs の WITH 句処理で集計演算が正しく行われていない。

具体的には以下の可能性が高い:
1. `crates/maharit-query/src/executor.rs` の WITH 句処理で、集計キー（non-aggregate expression）によるグルーピングが未実装または不完全
2. `execute_with_clause()` またはそれに相当する処理が、RETURN 句の集計関数とは別のコードパスを通っており、集計計算がスキップされている
3. WITH 句内の AggregateFunction の評価が None/null を返している

RETURN 句での集計（`MATCH (n:Person) RETURN COUNT(*)` など）は正常動作するため、WITH 句専用の処理に問題がある。

## 対応方針

1. `crates/maharit-query/src/executor.rs` の WITH 句処理（`apply_with_clause` または類似関数）を調査
2. WITH 句内の ReturnItem::Aggregate の評価ロジックを確認
3. グループキーによる集計グルーピング処理を実装または修正
4. `MATCH (n) WITH n.label AS k, COUNT(n) AS c RETURN k, c` 形式のユニットテストを追加

## 優先度
HIGH

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — WITH 句の実行処理
- `crates/maharit-query/src/ast.rs` — WithClause, WithItem の定義
- `scripts/query_feature_test.py` — 失敗テスト（test_with_pipeline）

## 解決済み (2026-04-15)

**根本原因**: `apply_with_clause` が各バインディングを個別にループして集計関数を評価していたため、GROUP BY が機能せずグループ集計値が null になっていた。

**修正内容**:
- `apply_with_clause` に集計関数検出を追加
- 集計ありの場合は `build_aggregated_result_set` を呼び出してグルーピング・集計を実行
- 結果を `Vec<Bindings>` に変換して返す
- `test_with_group_count`, `test_with_group_sum`, `test_with_aggregate_pipeline_then_return` テストを追加
