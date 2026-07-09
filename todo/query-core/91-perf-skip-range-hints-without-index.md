# query-core/91: 索引が無い場合は範囲ヒント収集をスキップ（範囲プッシュダウンのゼロコスト化）

## 概要
bug/87 の範囲プッシュダウンは `execute_query_segment` で WHERE 句ごとに
`collect_range_predicates` を実行し `range_hints` を構築していた。しかし
`range_index_candidates` は**プロパティ索引がある場合のみ**候補を絞れるため、
索引が 1 つも定義されていない環境（＝索引未作成の一般的なケース）では、
この収集は**発火し得ない純粋なオーバーヘッド**だった。

ベンチで範囲述語クエリ（`WHERE age > 40`, `WHERE id < 100`）が旧ベースライン比で
一貫して低め（run 間ノイズではなく安定して低い）だったため、この不要処理が
主因と判断。

## 対応
- `PropertyIndex::has_any_index()`（`!definitions.is_empty()` の O(1) 判定）を追加。
- `execute_query_segment`: `self.property_index.has_any_index()` が真のときのみ
  範囲ヒントを収集。索引が無ければ `range_hints` は空 Vec で即確定（収集も
  パース木走査もしない）。
- 範囲プッシュダウンの正当性・機能は不変（索引作成時のテスト 2 件は引き続き通過）。

## 効果
- 索引未作成環境での WHERE 範囲クエリのオーバーヘッドを排除（ゼロコスト abstraction）。

## ステータス
完了（core 150 / query 508 パス）。再ビルド後ベンチで範囲クエリの回復を確認予定。
