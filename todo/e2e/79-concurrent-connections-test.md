# Task 79: 同時接続テスト

## 背景・目的

複数クライアントが同時に接続・書き込みを行う状況でのデータ整合性と
サーバーの安定性を検証するテストがない。

`ConcurrentGraph` は DashMap による内部可変性で並行書き込みを
サポートしているが、実際に複数 TCP 接続から同時クエリを投げたときの
動作は未検証。

## 実装内容

### Python スクリプト: `scripts/concurrent_test.py`

#### テストケース

**同時書き込み**
```python
# 10スレッドが同時に異なるノードを作成
# → 全スレッド完了後、MATCH で10件存在すること
threads = [
    Thread(target=lambda: client.query(
        f"CREATE (n:Concurrent {{id: {i}}}) RETURN n"
    ))
    for i in range(10)
]
```

**同時読み書き混在**
```python
# 5スレッドが書き込み、5スレッドが読み取り
# → 読み取りがエラーやクラッシュなく完了すること
```

**max_connections 制限**
```python
# デフォルト max_connections=100 を超える接続を試みる
# → 超過分は接続拒否またはキューイングされること
```

**トランザクション分離**
```python
# 2つのクライアントが同時にトランザクションを開始
# → 互いに干渉しないこと（片方の未コミット変更が見えないこと）
```

### Rust テスト

`tcp_server.rs` に `#[tokio::test]` として追加：

```rust
#[tokio::test]
async fn test_concurrent_clients() {
    let server_addr = start_test_server().await;
    let handles: Vec<_> = (0..10).map(|i| {
        let addr = server_addr.clone();
        tokio::spawn(async move {
            let client = connect(addr).await;
            client.query(format!("CREATE (n:T {{i: {i}}})")).await
        })
    }).collect();
    for h in handles { h.await.unwrap().unwrap(); }
    // 全件確認
}
```

## 完了条件

- [x] 10 同時書き込みでデータ件数が正確であること
- [x] 読み書き混在でサーバーがクラッシュしないこと
- [x] max_connections 制限が機能すること
- [x] `cargo test` または `python3 scripts/concurrent_test.py` で実行可能

## 実装済み（Rust テスト in tcp_server.rs）

- `test_concurrent_writes`: 10タスクが同時にノードを作成 → 全件確認
- `test_concurrent_read_write_mix`: 5ライター + 5リーダー → クラッシュなし
- `test_max_connections_limit`: max_connections=3 に対して 6クライアント → サーバー生存確認
- `test_transaction_isolation`: A コミット・B ロールバック → A のノードのみ残ることを確認
