# Query Tuning

Source: https://neo4j.com/docs/cypher-manual/current/planning-and-tuning/query-tuning/

## 概要
- 目的は「必要なデータだけを取得する」こと。
- フィルタは早期に適用し、返すデータは最小限にする。
- 可変長パターンには上限を設ける。
- パラメータ利用でプラン再利用とキャッシュ効率を高める。

## 一般的な推奨
- 早期フィルタリング。
- ノード/関係の全返却を避け、必要なプロパティのみ返す。
- 可変長パターンに上限設定。

## クエリオプション
クエリ先頭に `CYPHER query-option ...` を付与して制御する。

### Planner
- `planner=cost`（既定）
- `planner=idp`（`cost`と同義）
- `planner=dp`（探索制限なし、計画時間増）

### connectComponentsPlanner（非推奨）
- `connectComponentsPlanner=greedy`
- `connectComponentsPlanner=idp`（既定）

### updateStrategy
- `updateStrategy=default`（既定）
- `updateStrategy=eager`

### expressionEngine
- `expressionEngine=default`（既定）
- `expressionEngine=interpreted`
- `expressionEngine=compiled`

### operatorEngine
- `operatorEngine=default`（既定）
- `operatorEngine=interpreted`
- `operatorEngine=compiled`（`runtime=slotted`と併用不可）

### interpretedPipesFallback
- `interpretedPipesFallback=default`（既定）
- `interpretedPipesFallback=disabled`
- `interpretedPipesFallback=whitelisted_plans_only`
- `interpretedPipesFallback=all`（実験的、結果不正の可能性）

### replanning
- `replan=default`（既定）
- `replan=force`
- `replan=skip`

### inferSchemaParts
- `inferSchemaParts=off`
- `inferSchemaParts=most_selective_label`
- 未指定時は `dbms.cypher.infer_schema_parts` 設定に従う。

## 関連リンク
- Execution plans: https://neo4j.com/docs/cypher-manual/current/planning-and-tuning/execution-plans/
- Operators: https://neo4j.com/docs/cypher-manual/current/planning-and-tuning/operators/
- Runtimes: https://neo4j.com/docs/cypher-manual/current/planning-and-tuning/runtimes/
