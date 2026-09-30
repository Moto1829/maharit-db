# タスク 118: `MATCH … SET … REMOVE …` の REMOVE 部分がエラーなく無視される

## 概要

`MatchSetStatement` は `set_clause` しか持たず（`crates/maharit-query/src/ast.rs`）、
SET の後ろに続く REMOVE 句はパースエラーにもならず黙って捨てられる。

```cypher
CREATE (:P {name: 'a', age: 1})
MATCH (n:P) SET n.age = 2 REMOVE n.name   -- 成功扱いだが n.name は残る
```

task116 のテスト作成時に発見。ユーザーから見ると「成功したのに反映されない」ため危険。

## 修正方針（案）

- パーサー: 文末に未消費トークンが残る場合はエラーにする（他の文でも同種の取りこぼしがないか確認）
- SET / REMOVE の任意の並び（`SET … REMOVE … SET …`）を `Vec<UpdateClause>` として保持・順に適用

## 受け入れ条件

- [x] `MATCH (n) SET … REMOVE …` で両方が適用される
- [x] 解釈できない後続トークンがある場合はパースエラーになる

## 完了内容 (2026-10-01)

### パーサー: 入力を最後まで解釈できなければエラー
`Parser::parse()` は文を 1 つ読んだ後、EOF でなければ `expected end of query` を返す（内部再帰は `parse_statement`）。
これにより **黙って捨てられていた構文** が表面化し、以下を修正した:

| 黙って無視されていた構文 | 影響 | 対応 |
|---|---|---|
| `MATCH … SET … REMOVE …` の REMOVE | 変更が反映されないのに成功 | `more_updates: Vec<UpdateClause>` で SET/REMOVE を記述順に適用（MatchSet / MatchRemove 両方） |
| `CREATE … RETURN …` の RETURN | 作成件数が返っていた（E2E・テストで多用） | `Statement::CreateReturn` を追加し RETURN を射影。作成エッジの変数も束縛 |
| `MATCH … CREATE … RETURN …` | 同上 | `MatchCreateStatement.return_clause` |
| `ORDER BY id(n)` / `toUpper(n.name)` 等 | `(n)` 以降が捨てられ別キーで並べていた | ORDER BY 項目を RETURN 項目と同じ文法に |

### 実行時に黙って無視されていたもの（同種の不具合として併せて修正）
- **ORDER BY のキーが返却列に無いと並べ替えをスキップ**（`RETURN n.name ORDER BY n.age`、`RETURN n ORDER BY n.name`）
  → 射影前のバインディングから評価して並べる。集計時に返却列でないキーはエラー（黙ってスキップしない）
- **非集計 WITH の ORDER BY が完全に無視**（SKIP/LIMIT だけ効いていた）→ DISTINCT → ORDER BY → SKIP → LIMIT の順で適用
- **WITH DISTINCT が重複を除去しない**: HashMap の Debug 文字列をキーにしており、`RandomState` によりマップ毎に走査順が違う → 変数名でソートしたキーに
- **単独 CREATE が同一句内で束縛済みの変数を再作成**（`CREATE (a:A), (a)-[:R]->(b:B)` で a が 2 つ）→ MATCH+CREATE と同じ実装に統一
- **UNWIND … CREATE … SET/RETURN が CREATE 前のバインディングを使用** → 作成後のバインディングを使う

### テスト
- パーサー: 全文種（19 パターン）で末尾ゴミがエラー / 正しい文 16 パターンが通る / SET・REMOVE の連鎖
- 実行: SET→REMOVE→SET、ORDER BY（非返却キー・関数式・集計時エラー）、WITH ORDER BY、WITH DISTINCT（3 列）、CREATE…RETURN（エッジ変数含む）、同一句の変数再利用、MATCH/UNWIND…CREATE…RETURN
- 修正前コードで実行し、追加した実行テスト 6 件がすべて失敗することを確認（WITH DISTINCT は 20 行 → 6 行を返していた）
- E2E 全通過（smoke 32 / concurrent 19 / constraint 26 / query_feature 63 / persistence 17 / replication 45 / failover 18）

### ドキュメント例の検査で判明（→ Cypher 適合テストで扱う）
docs の cypher ブロックを EXPLAIN でパース検査した結果、以下は **以前から動いていない**（今回の厳格化で後半が無視されずにエラーになったものを含む）:
- WHERE 内の関数呼び出し（`WHERE toLower(n.name) = "alice"`, `WHERE size(n.name) > 5`）
- `CALL proc(...) YIELD x AS y` / `YIELD … FOREACH …`
- `MATCH p = shortestPath(...)`
- `CREATE CONSTRAINT ON (n:L) ASSERT …`（旧構文）
- `CALL { … UNION … }`
