# タスク 181: openCypher TCK を取り込み、言語の意味を外部の基準で確かめる

## 背景（2026-11-16）
bug/176（NOT の優先順位）・bug/177（null の三値論理）・bug/178（不正な日付）は、
機能をタスク単位で「例のクエリが動けば完了」として足してきたため、言語全体にかかわる意味が
仕様に照らして確かめられていなかったことが原因。conformance_tests.rs（75 件）は自前で期待値を書いており、
fixture は比較するプロパティを全ノードが持っているので null の経路を通らない。

## 作業
- [ ] openCypher TCK（Apache 2.0）の feature ファイルを取り込む方法を決める（ベンダリング or ビルド時取得、ライセンス表記）
- [ ] Gherkin のシナリオ（Given グラフ / When クエリ / Then 結果・エラー）を読んで Executor で実行するテストランナーを作る
- [ ] まず expressions（比較・論理・null・算術・文字列・リスト）と temporal を対象にし、通らないシナリオは
      KNOWN_UNSUPPORTED と同じく一覧化（通るようになったら一覧から外せと失敗させる）
- [ ] 当面の穴埋めとして conformance_tests の fixture にプロパティが欠けたノードを足し、
      null と比較・OR・算術・IN、`NOT` と比較・IS NULL・STARTS WITH の組み合わせ、不正な日付の入力を追加
- [ ] CI に組み込み、通過件数を README か docs に載せる
