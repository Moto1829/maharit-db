# レプリケーション

MaharitDB は WAL ストリーミングベースのリーダー/フォロワー型レプリケーションをサポートします。

## アーキテクチャ概要

```
[Leader]
  ├── クライアントからの書き込みを受信
  ├── WAL に変更を記録
  └── WAL エントリをフォロワーにストリーミング

[Follower 1]     [Follower 2]
  ├── WAL を受信       ├── WAL を受信
  ├── ローカルに適用   ├── ローカルに適用
  └── 読み取り対応     └── 読み取り対応
```

## セットアップ

### リーダーの設定

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --enable-replication \
  --replication-role leader \
  --replication-port 7688
```

### フォロワーの設定

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --enable-replication \
  --replication-role follower \
  --leader-addr "leader-host:7688"
```

複数フォロワーの場合は、それぞれ別のポートで起動します：

```bash
# フォロワー 1
maharit server --port 7687 --replication-role follower --leader-addr "leader:7688"

# フォロワー 2
maharit server --port 7689 --replication-role follower --leader-addr "leader:7688"
```

## WAL ストリーミング

### ストリーミングの仕組み

リーダーはコミット済みの WAL エントリをフォロワーに非同期でストリーミングします。

```
Leader WAL:
  LSN 1001: CREATE (n:Person {name: "Alice"})
  LSN 1002: SET alice.age = 30
  LSN 1003: CREATE (a)-[:KNOWS]->(b)

Follower（リアルタイムで受信・適用）:
  LSN 1001: 適用済み
  LSN 1002: 適用済み
  LSN 1003: 受信中...
```

### レプリケーションラグの確認

```cypher
-- リーダーで実行（フォロワーの状態を確認）
CALL db.replication.status()
YIELD follower_address, lag_bytes, lag_ms, last_lsn
RETURN follower_address, lag_bytes, lag_ms
```

## ハートビートと生存監視

リーダーはフォロワーに定期的にハートビートを送信します。

- デフォルトのハートビート間隔: 5 秒
- フォロワーのタイムアウト閾値: 15 秒

```bash
maharit server \
  --replication-role leader \
  --heartbeat-interval 5s \
  --follower-timeout 15s
```

フォロワーがタイムアウトした場合：

```json
{"level":"WARN","message":"Follower timeout detected","follower":"follower-1:7687","last_heartbeat":"30s ago"}
{"level":"INFO","message":"Follower marked as disconnected","follower":"follower-1:7687"}
```

## フォロワーからの読み取り

フォロワーは読み取りクエリに対応しています。書き込みはリーダーに転送されます。

```rust
// フォロワーに接続して読み取り
let follower = Client::connect("follower-1:7687").await?;
let result = follower.query("MATCH (n:Person) RETURN n.name").await?;

// 書き込みはリーダーに接続
let leader = Client::connect("leader:7687").await?;
leader.execute("CREATE (n:Person {name: \"Alice\"})").await?;
```

### 読み取りの一貫性

フォロワーからの読み取りは「最終的一貫性（Eventual Consistency）」になります。レプリケーションラグがある場合、フォロワーはリーダーより古いデータを返すことがあります。

強い一貫性が必要な場合はリーダーから読み取ってください。

## スナップショット同期

新しいフォロワーを追加した場合、リーダーのスナップショットをフォロワーに転送して同期します。

```bash
# 自動スナップショット同期（フォロワー起動時に自動実行）
maharit server \
  --replication-role follower \
  --leader-addr "leader:7688" \
  --initial-sync snapshot  # デフォルト
```

## フェイルオーバー

リーダーが停止した場合の手動フェイルオーバー：

```bash
# フォロワーをリーダーに昇格
maharit admin promote-to-leader --addr follower-1:7687

# 古いリーダーをフォロワーとして再起動
maharit server \
  --replication-role follower \
  --leader-addr "follower-1:7688"  # 新しいリーダーのアドレス
```

自動フェイルオーバーは現在のバージョンでは未サポートです。外部の監視システム（例: Kubernetes Operator、Consul、Zookeeper）と組み合わせて実装してください。

## レプリケーション設定のまとめ

| 設定項目 | リーダー | フォロワー |
|---------|--------|---------|
| 書き込み | 対応 | 非対応（リーダーに転送） |
| 読み取り | 対応 | 対応（ラグあり） |
| ハートビート送信 | 送信 | 受信 |
| WAL ストリーミング | 送信 | 受信・適用 |
| ポート | 7687（クライアント）、7688（レプリケーション） | 7687（クライアント） |
