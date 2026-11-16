# タスク 180: docs の Cypher 例を実行して結果も確かめる

## 背景（2026-11-16）
`docs/cypher/where.md` に `WHERE NOT n.name = "Bob"` や `AND NOT n.deleted = true` が載っていたが、
`docs_cypher_examples` テストはパースできるかしか見ないため、間違った構文木（`(NOT n.name) = "Bob"`）でも
通り、bug/176 を見逃した。

## 作業
- [ ] docs の ```cypher ブロックの直後に ```text の出力例があるものは、テスト用データで実行して出力と比べる
      （データの用意の仕方を決める：ページ内の CREATE を先に流す / 共通の fixture を使う）
- [ ] 出力例が無い例は少なくとも実行してエラーにならないことを確かめる（書き込みはページごとの新しいグラフで）
- [ ] 実行できない・結果が合わない例は docs_cypher_known_failures.txt と同じ形の一覧に載せ、消化していく
- [ ] CLAUDE.md の「docs の Cypher 例」の説明を更新
