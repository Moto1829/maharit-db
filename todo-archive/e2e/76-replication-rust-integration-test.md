# Task 76: レプリケーション Rust 統合テスト

## 背景・目的

今回発生した不具合（フォロワーが TcpServer と別グラフを持ち、
WAL が適用されても MATCH クエリで取得できない）は Rust テストで
検出できなかった。

既存の `replication.rs` テストは WAL の送受信・handshake の
プロトコル動作しか確認しておらず、「フォロワーに対してクエリを投げ
リーダーと同じ結果が返るか」を検証するテストがない。

## 実装内容

`crates/maharit-server/src/replication.rs` または
`tests/replication_integration.rs` に `#[tokio::test]` として追加する。

### テストケース

#### データ伝播の基本確認
```
test_follower_receives_created_node
  1. リーダーを起動（LeaderReplicationManager + TcpServer を共有グラフで）
  2. フォロワーを起動（FollowerReplicationManager + TcpServer を共有グラフで）
  3. フォロワーをリーダーに接続
  4. リーダーに CREATE (n:Test {name: 'Alice'}) を実行
  5. 少し待機（WAL 伝播）
  6. フォロワーに MATCH (n:Test) RETURN n.name を実行
  7. "Alice" が返ること
```

#### プロパティの伝播
```
test_follower_receives_node_properties
  - 文字列・整数・真偽値プロパティを持つノードを作成
  - フォロワーで各プロパティを取得し一致すること
```

#### エッジの伝播
```
test_follower_receives_edge
  - リーダーでノード2つとエッジを作成
  - フォロワーでトラバーサルクエリが成功すること
```

#### 削除の伝播
```
test_follower_receives_deletion
  - リーダーでノードを作成 → 削除
  - フォロワーで MATCH が 0 件を返すこと
```

#### 共有グラフの参照確認
```
test_follower_and_server_share_same_graph
  - FollowerReplicationManager と TcpServer が
    Arc::ptr_eq で同一グラフを参照していること
```

## 実装方法

インプロセスでリーダー/フォロワーを起動し、ランダムポートで通信させる。
Docker 不要で CI でも実行できる。

```rust
#[tokio::test]
async fn test_follower_receives_created_node() {
    let graph = Arc::new(ConcurrentGraph::new());
    let leader_repl = setup_leader("127.0.0.1:0").await;
    let follower_repl = FollowerReplicationManager::with_concurrent_graph(
        follower_config(leader_repl.replication_addr()),
        Arc::clone(&graph),
    );
    let server = TcpServer::with_graph_arc(server_config(), Arc::clone(&graph));
    // ...
}
```

## 完了条件

- [x] フォロワーへのクエリでリーダー書き込みデータが取得できることを検証
- [x] プロパティ（文字列・数値）の伝播を検証
- [x] エッジの伝播を検証
- [x] 削除の伝播を検証
- [x] `cargo test -p maharit-server replication` が通ること
