# タスク 168: SET / REMOVE / DELETE が全文検索インデックスに反映されない

## 現象（2026-11-12、task139 で fulltext-index.md を確認中に発見）
- `SET a.body = '…'` で本文を変えても、全文検索は古い本文で一致する
- `DETACH DELETE` したノードが全文検索の結果に残る（node のプロパティは null）
- FOREACH の中の DELETE はプロパティインデックスからも外していなかった
（リーダーの実行経路の問題。フォロワーは schema::reindex_nodes で作り直していた）

## 対応
- [x] プロパティの変更（SET / REMOVE / SET +=・=）とラベルの変更で、そのノードの全文検索インデックスを作り直す
- [x] ノードの削除（DELETE / DETACH DELETE / FOREACH 内の DELETE）で全文検索・プロパティインデックスから外す
- [x] テスト（修正を外すと失敗することを確認）
