# クライアントライブラリ

## 概要
TCPサーバーに接続してクエリを実行するためのクライアントライブラリを実装する。

## 実装内容

### 接続管理
- [ ] TCPサーバーへの接続
- [ ] 接続のクローズ
- [ ] 自動再接続（オプション）
- [ ] 接続タイムアウト設定

### クエリ実行
- [ ] クエリの送信と結果の受信
- [ ] 結果のイテレータ/ストリーム対応
- [ ] エラーハンドリング

### API
- [ ] 同期API
- [ ] 非同期API（async/await）

### 接続プール（将来的）
- [ ] コネクションプーリング
- [ ] プールサイズ設定

## API例
```rust
// 同期API
let client = MaharitClient::connect("localhost:7687")?;
let result = client.query("MATCH (n:Person) RETURN n")?;
for row in result.rows() {
    println!("{:?}", row);
}

// 非同期API
let client = MaharitClient::connect_async("localhost:7687").await?;
let result = client.query("MATCH (n:Person) RETURN n").await?;
```

## 依存
- `12-tcp-server.md` が完了していること

## 対象クレート
新規 `maharit-client`
