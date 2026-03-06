# Python クライアント

maharit の Python クライアントライブラリは、MaharitDB に接続して Cypher クエリを実行するための純粋 Python 実装です。

## インストール

```bash
pip install maharit
# pandas 連携も使用する場合:
pip install maharit[pandas]
```

## 同期 API

`Client` クラスはソケットベースの同期クライアントです。

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # ノードの作成
    client.execute("CREATE (n:Person {name: 'Alice', age: 30})")
    client.execute("CREATE (n:Person {name: 'Bob', age: 25})")

    # クエリの実行
    result = client.query("MATCH (n:Person) RETURN n.name, n.age")
    for row in result:
        print(f"{row['n.name']}: {row['n.age']}")

    # pandas DataFrame に変換
    df = client.query("MATCH (n:Person) RETURN n.name, n.age").to_dataframe()
    print(df)
```

## 非同期 API

`AsyncClient` クラスは asyncio ベースの非同期クライアントです。

```python
import asyncio
from maharit import AsyncClient

async def main():
    async with AsyncClient.connect("localhost:7687") as client:
        await client.execute("CREATE (n:Person {name: 'Charlie'})")

        result = await client.query("MATCH (n:Person) RETURN n.name")
        for row in result:
            print(row["n.name"])

        # 大量結果のストリーミング
        async for row in client.stream("MATCH (n) RETURN n"):
            print(row)

asyncio.run(main())
```

## トランザクション

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # コンテキストマネージャーが自動でコミット/ロールバック
    with client.transaction() as tx:
        tx.execute("CREATE (a:Person {name: 'Alice'})")
        tx.execute("CREATE (b:Person {name: 'Bob'})")
        tx.execute(
            "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
            "CREATE (a)-[:KNOWS]->(b)"
        )
    # 例外が発生した場合は自動ロールバック
```

非同期トランザクション:

```python
async with AsyncClient.connect("localhost:7687") as client:
    async with client.transaction() as tx:
        await tx.execute("CREATE (n:Item {id: 1})")
        await tx.execute("CREATE (n:Item {id: 2})")
```

## ストリーミング

大量のデータを処理する場合はストリーミング API を使用します。

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # 1 件ずつ処理（メモリ効率が良い）
    for row in client.stream("MATCH (n) RETURN n", chunk_size=50):
        process(row)
```

## pandas DataFrame 連携

クエリ結果を直接 pandas DataFrame に変換できます。

```python
import pandas as pd
from maharit import Client

with Client.connect("localhost:7687") as client:
    df = client.query(
        "MATCH (n:Person) RETURN n.name AS name, n.age AS age"
    ).to_dataframe()

    print(df.head())
    print(df.dtypes)

    # pandas の機能をフル活用
    print(df[df["age"] > 25])
    print(df.groupby("name").mean())
```

## API リファレンス

### `Client`

| メソッド | 説明 |
|--------|------|
| `Client.connect(address, timeout=30.0)` | サーバーに接続 |
| `client.execute(query)` | Cypher クエリを実行 |
| `client.query(query)` | `execute` のエイリアス |
| `client.stream(query, chunk_size=100)` | 結果を 1 行ずつストリーム |
| `client.begin_transaction(read_only=False)` | トランザクション開始 |
| `client.transaction()` | トランザクション用コンテキストマネージャー |
| `client.ping()` | サーバーの疎通確認 |
| `client.stats()` | サーバー統計の取得 |
| `client.close()` | 接続を閉じる |

### `AsyncClient`

`Client` と同じ API ですが、すべてのメソッドが `async` です。
ストリーミングには `async for` を使用します。

### `QueryResult`

| メソッド | 説明 |
|--------|------|
| `result[i]` | インデックスでの行取得 |
| `len(result)` | 行数 |
| `for row in result` | 行のイテレーション |
| `result.to_dataframe()` | pandas DataFrame に変換 |

### `Transaction` / `AsyncTransaction`

| メソッド | 説明 |
|--------|------|
| `tx.execute(query)` | トランザクション内でクエリ実行 |
| `tx.commit()` | コミット |
| `tx.rollback()` | ロールバック |

## 例外クラス

| 例外 | 説明 |
|------|------|
| `MaharitError` | 基底例外クラス |
| `ConnectionError` | 接続エラー |
| `QueryError` | クエリ実行エラー |
| `TransactionError` | トランザクションエラー |

## プロトコル

クライアントは 4 バイトのビッグエンディアン長さプレフィックス + JSON ペイロードの形式で通信します。

```
[4-byte big-endian u32 length][JSON payload]
```

Python 標準ライブラリのみを使用しており、外部依存はありません（pandas は任意）。
