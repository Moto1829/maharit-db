# タスク: キーワードと衝突するノードラベル / リレーション型がパースエラーになる

## 概要

`User`, `Role`, `Index`, `Constraint` などの DDL / RBAC キーワードを
**ノードラベルとして**使用するクエリが Parse error になる。

```cypher
CREATE CONSTRAINT unique_user_email FOR (u:User) REQUIRE u.email IS UNIQUE
                                            ^^^^
        Parse error: expected identifier, found USER
```

Task 98 で「キーワードを識別子位置で識別子として再解釈する」修正を
**プロパティキー / プロパティアクセス / SET / REMOVE** には適用済みだが、
**ノードラベル位置** および **リレーション型位置** は対象外だった。

## 再現クエリ

```cypher
-- ❌ Parse error
CREATE (:User {email: 'foo@example.com'})
CREATE (:Role {name: 'admin'})
CREATE (:Index {name: 'idx_1'})
MATCH (u:User) RETURN u
MATCH (u)-[:User]->(v) RETURN u
CREATE CONSTRAINT c1 FOR (u:User) REQUIRE u.email IS UNIQUE
```

エラー例:
```
backend error: server error: Parse error: unexpected token:
  expected identifier, found USER at Span { ... }
```

回避策: ラベル名を変える (`User` → `Account`, `Role` → `UserRole` 等)。
Task 98 で確認した通り `Account` ラベルにすれば全機能が正常動作する。

## 検出されている影響

- `scripts/constraint_test.py` の `User` ラベルを使うテストが SHOW CONSTRAINTS=0 で
  失敗する（Task 96/97 の永続化バグの再発ではなく、本タスクの問題で
  CREATE CONSTRAINT 自体がパースエラーになっているため）
- Task 98 の `task_98_keyword_property_key_parse_error.md` と同根の「Cypher 標準では
  キーワードは予約語ではなく、コンテキスト依存に識別子として扱う」原則違反

## 根本原因の分析

`crates/maharit-query/src/parser.rs` のノードパターン / リレーションパターンの
ラベル読み取り箇所で、`self.expect_ident()?` を使っているためキーワードトークンが
受け入れられない。

該当箇所 (grep で確認可能):
- `parse_node_pattern()` — `(n:Label)` の Label 部
- `parse_relationship_pattern()` — `-[r:TYPE]->` の TYPE 部
- `parse_create_constraint()` — `FOR (var:Label)` の Label 部
- `parse_create_fulltext_index()` — `FOR (n:Label)` の Label 部
- `parse_show_indexes()` などラベル名を引数に取る場所

すべて Task 98 で追加した `self.expect_ident_or_keyword()?` に置き換えれば解決する見込み。

## 対応方針

### スコープ

ノードラベル / リレーション型 / 制約・インデックス定義のラベル引数すべて。
プロパティキーは Task 98 で対応済みなので除外。

### 実装

1. `parser.rs` のラベル / 型を読む全箇所を `expect_ident_or_keyword()` に置換
2. 既存テストで影響範囲を確認
3. 新規ユニットテスト:
   - `CREATE (:User {...})` がパース可能
   - `MATCH (u:User)-[:Role]->(r) RETURN u` がパース可能
   - `CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE` がパース可能
   - `CREATE FULLTEXT INDEX ... FOR (n:Index) ON (n.body)` がパース可能
4. E2E: `scripts/constraint_test.py` の `User` ラベルが期待通り動くことを確認

### 曖昧性の注意

`CREATE CONSTRAINT name FOR (u:User) REQUIRE u.email IS UNIQUE` のような
複合構文で、パーサーが `USER` を構文キーワードとして次に何を期待するかが
ラベル文脈ではなく別の文脈と紛れる可能性は低い (ラベルは必ず `:` の直後)。
パーサーのほうで context をすでに「ラベル位置」と分かっているので、
そこで `expect_ident_or_keyword()` を呼ぶのは安全。

ただし `CREATE USER alice WITH PASSWORD ...` のような RBAC DDL 構文と
パーサーの分岐先がぶつかる可能性は要確認（現状の dispatch を尊重するなら、
分岐先決定後の文脈でキーワードを許容するなら問題ない）。

## 優先度

MEDIUM（既存スクリプトの一部で実害があるが、回避策がある）

## 関連タスク

- Task 98 (`task_98_keyword_property_key_parse_error.md`) と同根、対応範囲の続編
- Task 96 (`task_96_constraint_enforcement_not_applied.md`) の永続化は無関係（解決済み）

## 関連ファイル

- `crates/maharit-query/src/parser.rs`
- `scripts/constraint_test.py` (現在 `User` ラベルを使用)
- `crates/maharit-server/src/auth.rs` (CREATE USER 構文の分岐参照)
