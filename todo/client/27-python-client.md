# Pythonクライアント

## 概要
PythonからMaharitDBに接続するためのクライアントライブラリを実装する。

## 実装内容

### 接続管理
- [ ] 同期API
- [ ] 非同期API（asyncio対応）
- [ ] コネクションプール
- [ ] 自動再接続

### クエリ実行
- [ ] クエリの実行と結果取得
- [ ] パラメータバインド
- [ ] トランザクション対応

### 結果の扱い
- [ ] Pythonオブジェクトへの変換
- [ ] pandas DataFrame対応
- [ ] イテレータ/ジェネレータ対応

### 型変換
- [ ] PropertyValue <-> Python型のマッピング
- [ ] Node/Edge のPython表現

## 実装方法
- [ ] PyO3によるRustバインディング
- [ ] または純粋Pythonでのプロトコル実装

## API例
```python
from maharit import Client

# 同期API
with Client.connect("localhost:7687") as client:
    client.execute("CREATE (n:Person {name: 'Alice'})")
    result = client.query("MATCH (n:Person) RETURN n.name")
    for row in result:
        print(row["n.name"])

# 非同期API
async with AsyncClient.connect("localhost:7687") as client:
    await client.execute("CREATE (n:Person {name: 'Bob'})")
    async for row in await client.query("MATCH (n) RETURN n"):
        print(row)

# pandas連携
df = client.query("MATCH (n:Person) RETURN n.name, n.age").to_dataframe()
```

## パッケージング
- [ ] PyPI公開（`pip install maharit`）
- [ ] ドキュメント（Sphinx）
- [ ] サンプルコード

## 依存
- `12-tcp-server.md` が完了していること

## 対象リポジトリ
`maharit-python`（別リポジトリ）または `clients/python`
