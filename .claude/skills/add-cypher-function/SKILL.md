---
name: add-cypher-function
description: MaharitDB の Cypher に組み込み関数（スカラー関数・集約関数・述語関数）を追加・変更するときに使う。実装する場所、null と引数の扱いの決まり、テスト・適合表・docs・既知の未対応一覧の更新までの手順。
---

# Cypher 関数を追加する

## 1. 実装（maharit-query）

スカラー関数は次の 4 か所すべてに足す。1 か所でも漏れるとコンパイルは通っても列名や EXPLAIN がおかしくなる。

| 場所 | 内容 |
|---|---|
| `src/ast.rs` の `ScalarFunction` | バリアントを追加（引数は `Box<Expression>`） |
| `src/parser/expr.rs` | 関数名（小文字）から `ScalarFunction` を作る分岐。引数の個数はここで検査する |
| `src/executor/functions.rs` の `evaluate_scalar_function` | 評価 |
| `src/executor/query.rs` の `function_to_name` と `src/executor/result.rs` の表示 | `RETURN f(x)` の列名・表示 |

集約関数は `AggregateFunction`（`ast.rs`）と `src/executor/aggregate.rs`。

関数名がキーワード（`lexer.rs` の `define_keywords!`）と同じなら、パーサーで個別に扱う必要がある（`all(...)` が例）。

## 2. 決まり

- 引数が `null` なら結果も `null`（Cypher の決まり。例外は `coalesce` など null を扱うための関数だけ）
- 型が合わない引数は `ExecuteError::TypeError("<関数名>() requires …")`。黙って `null` や別の値にしない
- 整数の演算は `checked_*` を使い、溢れたら `integer overflow` エラー（panic させない。リリースは panic=abort でサーバーごと落ちる）
- テスト以外で `unwrap()` は使えない（`#![deny(clippy::unwrap_used)]`）

## 3. テスト

1. `src/executor/tests/functions.rs`（集約は `read_basic.rs` など分野の合うファイル）に単体テスト。null の引数と型エラーの場合も書く
2. `src/conformance_tests.rs` の `cases()` に、標準の Cypher としての期待結果を 1〜数件足す
3. 以前は未対応だった関数なら `KNOWN_UNSUPPORTED`（同ファイル）から消す。残っていると `known_unsupported_queries_still_fail` が落ちる

## 4. docs

1. `docs/functions/<分類>.md`（日本語が正本）に説明と ```cypher の例を書く。例は `cargo test -p maharit-query --test docs_cypher_examples` でパースされる（1 段落 1 文、コメントは `//`）
2. 同じ変更を `docs/functions/<分類>.en.md` にも入れる（同じコミットなら翻訳の鮮度チェックは通る）
3. `crates/maharit-query/tests/docs_cypher_known_failures.txt` にその関数を使った例が載っていれば消す

## 5. 仕上げ

`bash scripts/check.sh` が `CHECK-OK` になってから、`feat(query): …` で 1 コミットにして push する。
既存の関数の結果を変える修正は `fix(query)!:`（CLAUDE.md の「後方互換を壊す変更」）。
