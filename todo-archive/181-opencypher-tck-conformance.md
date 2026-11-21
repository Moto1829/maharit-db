# タスク 181: openCypher TCK を取り込み、言語の意味を外部の基準で確かめる

## 背景（2026-11-16）
bug/176（NOT の優先順位）・bug/177（null の三値論理）・bug/178（不正な日付）は、
機能をタスク単位で「例のクエリが動けば完了」として足してきたため、言語全体にかかわる意味が
仕様に照らして確かめられていなかったことが原因。conformance_tests.rs（75 件）は自前で期待値を書いており、
fixture は比較するプロパティを全ノードが持っているので null の経路を通らない。

## 作業
- [x] openCypher TCK（Apache 2.0）の feature ファイルを取り込む方法を決める（ベンダリング or ビルド時取得、ライセンス表記）
- [x] Gherkin のシナリオ（Given グラフ / When クエリ / Then 結果・エラー）を読んで Executor で実行するテストランナーを作る
- [x] まず expressions（比較・論理・null・算術・文字列・リスト）と temporal を対象にし、通らないシナリオは
      KNOWN_UNSUPPORTED と同じく一覧化（通るようになったら一覧から外せと失敗させる）
- [x] 当面の穴埋めとして conformance_tests の fixture にプロパティが欠けたノードを足し、
      null と比較・OR・算術・IN、`NOT` と比較・IS NULL・STARTS WITH の組み合わせ、不正な日付の入力を追加
- [x] CI に組み込み、通過件数を README か docs に載せる

## 結果（2026-11-21）
- 取り込み方: ベンダリング。`crates/maharit-query/tests/tck/features/` に expressions の 10 領域（boolean / comparison / conditional / list / literals / mathematical / null / precedence / string / temporal）を commit 677cbafa のまま写し、`LICENSE`（Apache 2.0）と出どころを書いた README を置いた
- 実行器 `tests/tck.rs`: TCK が使う手順（Given / having executed / parameters / When / Then 結果・空・エラー）と Scenario Outline を読む。副作用の検査は見ない。エラーの種類（SyntaxError / TypeError …）は区別せず、エラーになったかだけを見る
- 1,742 シナリオ中 570 が通る。通らないものは `tests/tck_known_failures.txt`（通るようになると一覧から消せと失敗する）。`cargo test --workspace` に含まれるので CI でも動く
- 通過件数は docs/cypher/index.md の「標準への準拠」に載せた
- fixture の穴埋め（null・NOT・不正な日付の組み合わせ）は bug/176・177・178 の修正で conformance_tests と executor/tests に入れた
- 取り込みの途中で、40 段の入れ子のリストでスタックを使い切ってサーバーが落ちるバグを見つけて直した（fc8a7b03）
- 主な未対応（別タスクにする価値があるもの）: localdatetime / time / localtime、週・通日の日付、NaN の比較、リストの大小比較、範囲外のスライス、型の合わない添字をエラーにする、`IN` の右辺がリストでないときのエラー、16 進・8 進の整数リテラル
