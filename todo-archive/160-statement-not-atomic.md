# タスク 160: エラーになったクエリの途中までの変更が残る

## 現象（2026-11-12 確認）
```text
CREATE CONSTRAINT u_k FOR (n:U) REQUIRE n.k IS UNIQUE
UNWIND [1, 2, 2, 3] AS k CREATE (:U {k: k})
→ Execution error: unique constraint violation …
MATCH (n:U) RETURN count(n)   → 2
```
エラーになったクエリでも、エラーまでに作った 2 ノードが残る（トランザクション外でも内でも同じ）。
Cypher では 1 つの文は全部成功するか何もしないかのどちらか。

## 作業（案）
- [ ] トランザクション外のクエリも undo ログを取り、エラーなら打ち消してから応答する（`execute_in_tx` の undo を流用）
- [ ] WAL・レプリケーションには打ち消し後の結果だけが流れるようにする（または打ち消しの変更も流す）
- [ ] テスト、docs/advanced/transactions.md の「既知の制限」を更新
