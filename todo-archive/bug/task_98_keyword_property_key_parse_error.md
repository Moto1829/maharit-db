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

## 解決済み (2026-06-14)

### 実装内容

1. **`crates/maharit-query/src/lexer.rs`**
   - `keyword_as_ident(&TokenKind) -> Option<&'static str>` 関数を追加
   - 全 58 個のキーワード TokenKind に対して小文字の識別子名を返すマップを実装
2. **`crates/maharit-query/src/lib.rs`**
   - `keyword_as_ident` を pub re-export
3. **`crates/maharit-query/src/parser.rs`**
   - 既存の `expect_ident_or_keyword()` を全キーワード対応に拡張
     （以前は `Index` / `Fulltext` のみ受け入れる限定版だった）
   - `parse_property_key()` ヘルパーを新規追加（map リテラルのキー位置で利用）
   - ドットアクセス後のプロパティ名読み取りを 8 箇所で `expect_ident_or_keyword()` に置換:
     - `SET n.prop = val`
     - `RETURN n.prop`
     - `ORDER BY n.prop`
     - `REMOVE n.prop`
     - `REQUIRE n.prop IS ...` (複合キー / 単一キー)
     - `FULLTEXT INDEX ... ON (n.prop)`
     - `Expression::Property` 評価

### 適用範囲

| 位置 | 修正前 | 修正後 |
|------|--------|--------|
| map リテラルのキー (`{role: 1}`) | ❌ Parse error | ✅ OK |
| `n.role` プロパティ参照 | ❌ Parse error | ✅ OK |
| `WHERE n.role = 'x'` | ❌ Parse error | ✅ OK |
| `SET n.role = 'x'` | ❌ Parse error | ✅ OK |
| `REMOVE n.role` | ❌ Parse error | ✅ OK |
| `ORDER BY n.role DESC` | ❌ Parse error | ✅ OK |
| エッジプロパティ `[:R {role: 'x'}]` | ❌ Parse error | ✅ OK |
| 制約宣言 `REQUIRE n.role IS UNIQUE` | ❌ Parse error | ✅ OK |
| フルテキストインデックス `FOR (n.role)` | ❌ Parse error | ✅ OK |

### テスト

- `crates/maharit-query/src/parser.rs::tests::` に 5 件追加:
  - `test_parse_property_key_keyword_role`
  - `test_parse_property_key_keyword_in_relationship`
  - `test_parse_property_key_multiple_keywords`
  - `test_parse_property_key_keyword_uppercase`
  - `test_parse_property_access_keyword`
- `cargo test -p maharit-query` → **487/487 PASS**（既存 482 + 新規 5）
- ワークスペース全体テスト: maharit-storage の WAL テスト 2 件が並列実行で
  flaky だがシリアル実行で PASS（既知の問題、本タスクとは無関係）

### Docker 環境での検証

`docker compose up -d --build` で再デプロイし、Web UI 経由で動作確認:

- `CREATE (n {role: 'x', user: 'u', password: 'p', type: 't', index: 1})` → OK
- `MATCH (n) RETURN n.role, n.user, n.password, n.type, n.index` → 正しい値が返る
- `MATCH (n) WHERE n.role = 'Admin' RETURN n` → OK
- `SET n.role = 'SuperAdmin'` → 更新成功
- `[:WORKS_AT {role: 'CTO', type: 'fulltime'}]` → エッジ作成 + 取得成功
