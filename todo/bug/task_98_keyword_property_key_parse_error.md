# タスク: キーワードと衝突する識別子をプロパティキーに使えない

## 概要

`role` などの RBAC・DDL 系キーワードを map リテラルのキー（プロパティ名）に
使用すると Parse error が発生する。Cypher 標準では `role` は予約語ではなく、
プロパティ名として使えるべき。Neo4j も同様にコンテキスト依存でキーワードを
識別子として扱う。

## 再現クエリ

```cypher
MATCH (a:Person {id:'alice'}), (co:Company {id:'acme'})
CREATE (a)-[:WORKS_AT {role: 'Engineer'}]->(co)
```

エラー:
```
backend error: server error: Parse error: unexpected token:
  expected identifier, found ROLE at Span { start: 79, end: 83, line: 1, column: 80 }
```

## 根本原因の分析

`crates/maharit-query/src/lexer.rs` の `lookup_keyword()` が `role`/`ROLE`/`Role`
を一律で `TokenKind::Role` に分類している。
`crates/maharit-query/src/parser.rs` の `parse_properties()` (line 2414) は
プロパティキー位置で `TokenKind::Ident(_)` または `TokenKind::String(_)`
しか受け付けず、キーワードトークン（`Role`, `Type`, `User`, `Name` 等）が来ると
`expected identifier` でエラーになる。

他にも `name` / `count` / `type` / `user` / `password` などのよく使われる
プロパティ名候補が同様の問題を抱えている可能性が高い。

## 対応方針

### スコープ

プロパティキー位置（map リテラルのキー）のみ。
ノードラベル / リレーションシップタイプ / RETURN エイリアスは別タスク化する
（今回はユーザー体験を最も損ねている map リテラルだけを対象）。

### 実装

1. `parser.rs` に `parse_property_key()` ヘルパーを追加
   - `TokenKind::Ident(s)` → `s`
   - `TokenKind::String(s)` → `s`
   - その他のキーワードトークン → 小文字表記の名前を返す
     （例: `TokenKind::Role` → `"role"`、`TokenKind::User` → `"user"`）
2. `parse_properties()` のキー読み取りを `parse_property_key()` 経由に変更
3. 同様にプロパティキーを読む他箇所も置き換え可能なら置き換える
4. ユニットテストを追加:
   - `CREATE (n {role: 'x'})` がパース可能
   - `{role: 1, type: 'A', name: 'n', count: 2, user: 'u'}` のような複合キー
   - 既存テスト (quoted keys / mixed keys) が壊れていない

## 優先度

MEDIUM（回避策あり: 別名を使う / バッククォートで囲む等）

## 関連ファイル

- `crates/maharit-query/src/parser.rs` (`parse_properties` / `expect_ident`)
- `crates/maharit-query/src/lexer.rs` (keyword 一覧)
