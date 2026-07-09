# query-core/87: WHERE 範囲述語の索引プッシュダウン + プロパティ索引メンテナンス（性能改善 7）

## 概要
2 つの密結合した変更をまとめる。

### (A) プロパティ索引メンテナンスの正当性修正（前提・必須）
`apply_set_clause`(SET) / REMOVE / DELETE は `set_node_property` 等でグラフを
変更するのに `property_index` を更新していなかった。query-core/85 で索引を
リクエスト間で共有・永続化したことにより、「SET 後に索引が陳腐化 → 索引ベースの
候補絞り込み（既存の等価索引パス `find_by_property` を含む）が偽陰性で誤結果」という
潜在バグが顕在化し得る状態になっていた。

- `Executor::reindex_node_property(node, prop, old, new)` を追加。
  `(primary_label, prop)` に索引がある時のみ旧値を除去し新値を登録。
- SET(単一/MergeProperties) の後に旧値を退避して reindex。
- REMOVE property の後に旧値を unindex。
- DELETE node の後に `property_index.remove_node`。

### (B) WHERE 範囲述語の索引プッシュダウン
`WHERE n.age > 40` 等（Lt/Gt/Lte/Gte）は索引にも落ちず全ラベルスキャンだった。
`property_index` は範囲 API（`find_by_int_range`/`find_by_float_range`）を持つのに未活用。

- `collect_range_predicates`: WHERE トップレベル AND の `var.prop <op> <数値>` を収集
  （プロパティが右辺なら演算子を反転）。
- `execute_query_segment`(&mut self 化) がセグメントの範囲述語を `range_hints` に退避し、
  ノードスキャン後にクリア（他クエリ・サブクエリへ漏らさない）。
- `range_index_candidates`: 索引がある数値プロパティについて、演算子に応じた
  **候補スーパーセット**を int/float 両範囲索引の union で取得。
  型混在（int/float）でも取りこぼさない。境界外は i64 にクランプ。
- `match_node_pattern` の候補集合を「範囲索引 → ラベル索引 → 全ノード」の優先度に。
- 正当性は `node_matches_pattern` と WHERE の retain が保証（スーパーセットで安全）。

## 効果
- `CREATE INDEX` 済みプロパティへの範囲クエリが O(全ラベル走査) → 索引での候補絞り込みに。
- 併せて索引の正確性を SET/REMOVE/DELETE で担保（query-core/85 の副作用を解消）。

## テスト
- `test_property_index_updated_on_set_and_delete`（SET/DELETE で索引が追随）
- `test_range_pushdown_with_index`（>, >=, <, <=, 左辺リテラル反転, SET 後維持）
- `test_range_pushdown_mixed_int_float`（int/float 混在で union が取りこぼさない）

## ステータス
完了（query 508、workspace 全16バイナリパス）
