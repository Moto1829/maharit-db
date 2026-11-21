# maharit-query

Cypher のクエリパイプライン: **lexer → parser → planner → executor**。句・関数の追加とバグ修正の手順はスキル `add-cypher-clause` / `add-cypher-function` / `fix-bug`。

## ファイル構成

- `ast.rs`: `Statement`（トップレベルの文）と `Expression`。`is_read_only(&Statement)` が読み取り専用の判定（サーバーはこれで実行経路とロールの検査を決める）
- `lexer.rs`: キーワードは `define_keywords!`。大文字小文字を区別しない
- `parser.rs` + `parser/`: 再帰下降。`parser.rs` に `Parser` 本体・文の振り分け（`parse_statement`）・トークン操作。`statement.rs`（句と文）/ `expr.rs`（式。優先順位は `OR` < `XOR` < `AND` < `NOT` < 比較 < 加減 < 乗除 < 単項）/ `pattern.rs` / `schema.rs`（制約・索引・ユーザー・ACL）/ `subquery.rs`
- `planner.rs`: EXPLAIN / PROFILE 用の `QueryPlan`。実行はプランナを通らない（executor が直接 AST を実行する）
- `executor.rs` + `executor/`: `executor.rs` に `Executor` 本体と文の振り分け（`execute`）。`query.rs`（読み取り文・WITH・押し下げ）/ `match_.rs`（パターン照合）/ `write.rs` / `expr.rs` / `functions.rs` / `aggregate.rs` / `result.rs`（RETURN・ORDER BY・LIMIT）/ `schema.rs` / `procedure.rs` / `algo.rs` / `explain.rs`。型は `value.rs` / `bindings.rs` / `error.rs`
- `acl.rs`: ACL を適用する `GraphBackend` のラッパー（`AclBackend`）
- `cache.rs`: `AstCache` / `PlanCache`

ファイルをまたいで呼ぶメソッドは `pub(super)`。クレートの外へは広げない。

## 知らないと間違えること

- `Executor` はグラフを生ポインタで持ち、`unsafe impl Sync` している。成り立つ前提: `new_readonly`（`readonly: true`）で作った Executor からは `graph_mut()` を呼ばない。書き込みは `is_read_only` が false を返す文の経路だけに置く（サーバーは `is_read_only` で読み取り用の経路を選ぶ）
- サーバーの読み取りは `new_concurrent_shared`（制約・索引のマネージャを共有参照で持つ）。そこでマネージャを変えると copy-on-write でその Executor の中だけの変更になり、サーバーには残らない
- グラフは必ず `graph_ref()` / `graph_mut()` 経由で触る。ACL が有効だと、これが `AclBackend` を返す（直接 `self.graph` を読むと ACL を素通りする）
- プロパティを変える経路（SET / REMOVE / DELETE / MERGE の更新）では `reindex_node_property` を呼ぶ。漏れるとプロパティ索引が古くなり、索引を使う MATCH が行を取りこぼす
- ラベルの変更は `add_node_label` / `remove_node_label` を通す（ラベル索引を更新する版）
- パターンに束縛済みの変数があれば、照合し直して上書きしない（bug/125）。CREATE でも新しく作らずそのノードを使う
- 名前を読む位置（ラベル・型・プロパティ名）では `expect_ident_or_keyword` を使う。キーワードと同じ綴りでも書かれたとおりに読む（bug/175）
- `all` はキーワード（`TokenKind::All`）なので、関数 `all(...)` はパーサーで個別に扱っている。キーワードと同名の関数を足すときは同じ対応が要る
- null: `=` などの比較は `cypher_equals`（null なら `None`）。`values_equal` は null 同士も等しいとみなす版で、DISTINCT・グループ分け用（bug/177）
- 整数の演算は `checked_*` で溢れをエラーにする。テスト以外で `unwrap()` は使えない（`#![deny(clippy::unwrap_used)]`）

## テスト

- 単体テストは各モジュールの `tests/`（`executor/tests/` など）に分野ごと。ヘルパーは `tests/mod.rs`（`execute(&mut graph, q)`、状態を持つ Executor が要るときは `execute_with`）
- 結果の順序はハッシュに依存するので、ORDER BY の無いクエリは並べ替えてから比べる
- 標準の Cypher としての意味は `conformance_tests.rs` の表と、openCypher TCK（`tests/tck.rs`、シナリオは `tests/tck/features/`）で確かめる。TCK の未対応は `tests/tck_known_failures.txt`。対応して通るようになると一覧から消せとテストが失敗する（作り直しは `MAHARIT_TCK_WRITE=1 cargo test -p maharit-query --test tck`）
- docs の ```cypher は `tests/docs_cypher_examples.rs` でパースされる。未対応は `tests/docs_cypher_known_failures.txt`
- 性能は `benches/engine.rs`（`cargo bench -p maharit-query --bench engine -- <名前>`）
