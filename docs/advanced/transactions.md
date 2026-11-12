---
title: トランザクション
parent: 高度なトピック
nav_order: 1
---

# トランザクション

`maharit server` では、複数のクエリを 1 つのトランザクションにまとめ、最後にコミットするか、ロールバックしてまとめて取り消せます。トランザクションは Cypher の文（`BEGIN` / `COMMIT`）ではなく、[TCP プロトコル](../api/protocol.md)の `begin` / `commit` / `rollback` リクエストで操作します。

## クライアントから使う

### Python

```python
from maharit import Client

with Client.connect("127.0.0.1:7687") as client:
    # ブロックを抜けるとコミット、例外が出るとロールバック
    with client.transaction() as tx:
        tx.execute("MATCH (a:Account {id: $id}) SET a.balance = a.balance - $amount",
                   {"id": "A", "amount": 100})
        tx.execute("MATCH (b:Account {id: $id}) SET b.balance = b.balance + $amount",
                   {"id": "B", "amount": 100})
```

`client.begin_transaction()` で `Transaction` を受け取り、`tx.commit()` / `tx.rollback()` を自分で呼ぶこともできます。

### Rust

```rust
let tx = client.begin().await?;
client.query_in_tx("MATCH (a:Account {id: 'A'}) SET a.balance = a.balance - 100", Some(tx)).await?;
client.query_in_tx("MATCH (b:Account {id: 'B'}) SET b.balance = b.balance + 100", Some(tx)).await?;
client.commit(tx).await?;      // 取り消すときは client.rollback(tx)
```

### プロトコル

```json
{"type": "begin"}
{"type": "query", "query": "CREATE (:Item {id: $id})", "params": {"id": 1}, "txId": 7}
{"type": "commit", "txId": 7}
```

`begin` の応答 `{"type": "transactionBegun", "txId": 7}` の `txId` を、トランザクション内のクエリと `commit` / `rollback` に付けます。

## 動作

トランザクション内の書き込みは、**実行した時点でグラフと WAL に反映**されます。ロールバックは、それまでの変更を打ち消す変更を適用して元に戻します。

- **他の接続から見える**: コミット前の変更も、他の接続のクエリからすぐに見えます（分離レベルとしては Read Uncommitted に相当）。ロールバックすると見えなくなります
- **書き込みは 1 つずつ**: サーバーの書き込みは全接続で 1 つずつ順に実行されます。同じデータへの同時書き込みで競合エラーになることはなく、後から実行した書き込みが勝ちます
- **コミット**: トランザクションを終えるだけです（変更は既に反映済み）
- **ロールバック**: ノード・リレーションシップの作成と削除、プロパティとラベルの変更、インデックスを元に戻します。打ち消す変更も WAL に書かれるので、再起動やフォロワーでも取り消された状態になります
- **接続が切れたとき**: コミットもロールバックもしていないトランザクションは、自動でロールバックします
- **読み取り専用**: `{"type": "begin", "readOnly": true}`（Python の `client.transaction(read_only=True)`）で始めたトランザクションでは、書き込みのクエリはエラーになります
- **1 つのクエリは全部か無しか**: クエリが途中でエラーになると（制約違反など）、そのクエリが行った変更はすべて取り消されます。トランザクション内なら、それより前のクエリの変更は残ります
- **障害時**: コミット前にサーバーが落ちた場合、再起動後もそれまでの変更は残ります（自動ではロールバックされません）

ロールバックで確実に取り消したい処理以外は、トランザクションを使わずに 1 つのクエリで書くと簡単です。1 つのクエリの中で複数の要素を作ったり更新したりできます。

```cypher
MATCH (a:Account {id: "A"}), (b:Account {id: "B"})
SET a.balance = a.balance - 100, b.balance = b.balance + 100
```

## 大量のデータを書き込む

1 件ずつクエリを送るより、リストのパラメータを `UNWIND` で展開して 1 つのクエリで書き込むほうが速くなります。

```cypher
UNWIND $batch AS item
CREATE (n:Person {name: item.name, age: item.age})
```

1 回に送るのは 1,000〜10,000 件程度が目安です。WAL への書き込み（fsync）のまとめ方は [サーバー設定](../operations/server-config.md) を参照してください。
