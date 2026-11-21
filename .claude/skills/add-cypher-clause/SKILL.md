---
name: add-cypher-clause
description: MaharitDB の Cypher に新しい句・文・演算子・キーワード（例: CALL の新しい形、SHOW …、新しい WHERE 述語）を追加するときに使う。AST・字句解析・構文解析・実行・プランナ・読み取り専用の判定・docs までの手順と、既存のクエリを壊さないための確認。
---

# Cypher の句・文を追加する

## 1. 順番（すべて crates/maharit-query/src）

1. `ast.rs`: `Statement` か句の型を追加
2. `lexer.rs`: キーワードを `define_keywords!` に足す（`"FOO" => Foo => "foo"`）
3. `parser/`: 文なら `parser.rs` の `parse_statement` の分岐と `statement.rs`、式なら `expr.rs`、パターンなら `pattern.rs`、管理系の文なら `schema.rs`
4. `executor/`: 実行は機能の合うファイル（`query.rs` / `write.rs` / `schema.rs` …）。文の振り分けは `executor.rs` の `execute`
5. `planner.rs`: EXPLAIN / PROFILE で表示するノード
6. `ast.rs` の `is_read_only`: 読み取り専用かどうか。書き込む文を true にするとサーバーが読み取り用の経路（共有ロック、reader ロールで許可）で実行してしまう
7. サーバーで特別に扱う文（ユーザー管理・ACL など）は `crates/maharit-server/src/tcp_server/session.rs` も

## 2. キーワードを増やすときの確認

キーワードにすると、その単語をラベル・プロパティ名・変数名に使っている既存のクエリが壊れうる。

- ラベル・型・プロパティ名の位置では `expect_ident_or_keyword` を使っているので、書かれたとおりの綴りで読める（bug/175）。新しく名前を読む箇所でも `expect_ident` ではなくこれを使う
- 変数名や関数名に使われていないか、`docs/` と `scripts/*.py` を grep する
- 演算子なら優先順位を確かめる（`NOT` は AND と比較の間、bug/176）

## 3. テストと docs

- 構文解析のテストは `parser/tests/`、実行は `executor/tests/` の分野の合うファイル
- 標準の Cypher なら `conformance_tests.rs` に期待結果を足し、`KNOWN_UNSUPPORTED` にあれば消す
- `docs/cypher/` に説明と例（日本語と `.en.md` の両方）。`docs_cypher_known_failures.txt` に該当する例があれば消す
- E2E（`scripts/query_feature_test.py`）に足すと、TCP 経由でも確かめられる

## 4. 仕上げ

`bash scripts/check.sh` → `feat(query): …` で 1 コミット → push。既存のクエリの意味が変わるなら `!` を付ける。
