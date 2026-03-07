# Pythonクライアント

## 概要
PythonからMaharitDBに接続するためのクライアントライブラリを実装する。

## 実装内容

### 接続管理
- [x] 同期API
- [x] 非同期API（asyncio対応）
- [x] コネクションプール
- [x] 自動再接続

### クエリ実行
- [x] クエリの実行と結果取得
- [x] パラメータバインド
- [x] トランザクション対応

### 結果の扱い
- [x] Pythonオブジェクトへの変換
- [x] pandas DataFrame対応
- [x] イテレータ/ジェネレータ対応

### 型変換
- [x] PropertyValue <-> Python型のマッピング（JSON経由）
- [x] Node/Edge のPython表現（専用クラス）

## 実装方法
- [x] PyO3によるRustバインディング（clients/python-native/ に実装）
- [x] または純粋Pythonでのプロトコル実装

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
- [x] PyPI公開（`pip install maharit`）（pyproject.toml 設定済み）
- [x] ドキュメント（Sphinx）（clients/python/docs/ に作成済み）
- [x] サンプルコード（README.md）

## 依存
- `12-tcp-server.md` が完了していること

## 対象リポジトリ
`maharit-python`（別リポジトリ）または `clients/python`
