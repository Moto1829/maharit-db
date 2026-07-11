# query-core/83: WHERE 述語の索引/スキャンプッシュダウン（性能改善 4/4）

## 概要
`WHERE n.city = 'Tokyo'` は全ラベル一致ノードを実体化した後に `retain` で
後置フィルタしていた（インラインの `{city:'Tokyo'}` は索引を使うのに WHERE は使わない）。
このため WHERE 等価がフルスキャンより遅い現象が起きていた。

## 対応
- `execute_query_segment` に WHERE のフィルタプッシュダウンを追加。
  - `collect_pushable_equalities`: WHERE のトップレベルで AND 連結された
    `var.prop = <定数>`（リテラル/パラメータ）を収集。OR/NOT の下には降りない。
  - `augment_patterns` / `augment_node_pattern`: 対応する変数のノードパターンに
    その等価述語をプロパティ制約として注入（既存インラインは上書きしない）。
  - これにより `match_node_pattern` の既存の索引パス（`has_index` かつリテラル）が
    発火、索引が無い場合もスキャン中に絞り込まれ、非マッチ binding の複製を回避。
- 元の WHERE `retain` は据え置き（押し下げは候補の早期枝刈りのみで意味論は不変）。
- OPTIONAL MATCH には押し下げない（NULL 保持行を誤って除外しないため）。
- 定数以外（他変数参照）は束縛順の問題があるため対象外。

## 効果
- `MATCH (n:L) WHERE n.p = <定数>` が索引利用（索引あり）または
  スキャン中フィルタ（索引なし）になり、全件実体化を回避。
- 正当性テスト追加（等価/OR非プルーニング/AND範囲併用/インライン同義一致）。

## ステータス
完了（query 501 テスト、workspace 全16バイナリパス）
