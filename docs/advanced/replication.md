---
title: レプリケーション
parent: 高度なトピック
nav_order: 2
---

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
  --host 0.0.0.0 --port 7687 \
  --replication-role leader \
  --replication-bind 0.0.0.0:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET" \
  --node-id leader
```

### フォロワーの設定

```bash
maharit server \
  --host 0.0.0.0 --port 7687 \
  --replication-role follower \
  --leader-addr leader-host:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET" \
  --node-id follower-1
```

`--node-id` はクラスター内で一意にします。同じホストで複数のフォロワーを動かす場合は、`--port`・`--node-id`・`--metrics-bind` をそれぞれ変えてください。

| オプション | 対象 | 説明 |
|-----------|------|------|
| `--replication-role` | 両方 | `leader` / `follower` |
| `--replication-bind` | リーダー | レプリケーションの待ち受けアドレス（デフォルト `127.0.0.1:7688`） |
| `--leader-addr` | フォロワー | リーダーのレプリケーションアドレス（必須） |
| `--replication-secret` | 両方 | 共有シークレット（環境変数: `MAHARIT_REPLICATION_SECRET`）。設定すると、リーダーは同じシークレットを示さないフォロワーを拒否する |
| `--leader-ca` | フォロワー | リーダーの証明書を検証する CA 証明書。指定すると TLS で接続する（環境変数: `MAHARIT_LEADER_CA`） |
| `--node-id` | 両方 | ノード ID（環境変数: `MAHARIT_NODE_ID`、デフォルト `node-1`） |

## 通信の暗号化（TLS）

リーダーを `--tls-cert` / `--tls-key` 付きで起動すると、クライアント接続（7687）に加えてレプリケーション接続（7688）も TLS になり、平文のフォロワーは接続できなくなります。フォロワーには `--leader-ca` で、リーダーの証明書を検証する CA 証明書（自己署名ならリーダーの証明書そのもの）を指定します。

```bash
# リーダー
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/leader.crt --tls-key /etc/maharit/leader.key \
  --replication-role leader --replication-bind 0.0.0.0:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET"

# フォロワー
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/follower.crt --tls-key /etc/maharit/follower.key \
  --replication-role follower --leader-addr leader.example.com:7688 \
  --leader-ca /etc/maharit/ca.crt \
  --replication-secret "$MAHARIT_REPLICATION_SECRET"
```

- リーダーの証明書は、フォロワーの `--leader-addr` に書いたホスト名（または IP アドレス）に対して有効である必要があります。
- 共有シークレットは TLS の中で送られます。TLS なしでは、シークレットも複製されるデータも平文でネットワークを流れるので、盗聴されるとフォロワーになりすまされます。TLS なしでリーダーがループバック以外に待ち受けると、起動時に警告が出ます。
- フォロワー自身の `--tls-cert` / `--tls-key` は、フォロワーへのクライアント接続を暗号化するためのものです（省略可）。

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

### レプリケーションの状態を確認する

各ノードの `stats` リクエスト（Python の `client.stats()`）の `replication` に、ロール・ノード ID・適用済みの LSN・接続中のフォロワー数（リーダー）・リーダーの生存（フォロワー）が入ります。リーダーとフォロワーの `current_lsn` の差が、未適用の変更の数です。

```python
from maharit import Client

with Client.connect("follower-1:7687") as client:
    print(client.stats()["replication"])
    # {'role': 'follower', 'node_id': 'follower-1', 'current_lsn': 1003,
    #  'follower_count': 0, 'is_leader_alive': True}
```

## ハートビートと生存監視

リーダーは 1 秒ごとにフォロワーへハートビートを送ります。フォロワーは 5 秒間ハートビートが届かないと、リーダーが停止したとみなします（`is_leader_alive` が `false` になる）。間隔は変更できません。

## フォロワーからの読み取り

フォロワーには読み取りクエリだけを送ってください。書き込みはリーダーに送ります（フォロワーは書き込みを拒否しないので、送るとリーダーとデータがずれます）。

```rust
// フォロワーに接続して読み取り
let mut follower = Client::connect("follower-1:7687").await?;
let result = follower.query("MATCH (n:Person) RETURN n.name").await?;

// 書き込みはリーダーに接続
let mut leader = Client::connect("leader:7687").await?;
leader.execute("CREATE (n:Person {name: \"Alice\"})").await?;
```

### 読み取りの一貫性

フォロワーからの読み取りは「最終的一貫性（Eventual Consistency）」になります。レプリケーションラグがある場合、フォロワーはリーダーより古いデータを返すことがあります。

強い一貫性が必要な場合はリーダーから読み取ってください。

## 既存データの同期

!!! warning "スナップショット同期は未実装"
    `maharit server` のリーダーは、新しく接続したフォロワーに既存のデータを送りません。フォロワーが受け取るのは、接続した後の変更だけです。データのあるリーダーにフォロワーを追加するときは、リーダーを停止してデータファイル（`--data`）をフォロワーにコピーしてから、両方を起動してください。

## フェイルオーバー

!!! warning "昇格コマンドは機能しない"
    `maharit admin promote-to-leader` は終了コード 0 を返しますが、フォロワーの状態は変わりません（昇格の指示がフォロワーに届かず、サーバーも参照していません）。また、フォロワーは書き込みクエリを拒否せずにそのまま受け付けるため、フォロワーに書き込むとリーダーとデータがずれます。

リーダーが停止したときは、次の手順で切り替えてください。

1. 新しいリーダーにするフォロワーを停止する（データファイルが保存される）
2. そのデータファイルを使い、`--replication-role leader` で起動し直す
3. 残りのフォロワーを、データファイルを新しいリーダーのものに置き換えてから、`--leader-addr` を新しいリーダーにして起動し直す

自動フェイルオーバーには対応していません。

## レプリケーション設定のまとめ

| 設定項目 | リーダー | フォロワー |
|---------|--------|---------|
| 書き込み | 対応 | 拒否しない（リーダーには反映されない。書き込まないこと） |
| 読み取り | 対応 | 対応（ラグあり） |
| ハートビート送信 | 送信 | 受信 |
| WAL ストリーミング | 送信 | 受信・適用 |
| ポート | 7687（クライアント）、7688（レプリケーション） | 7687（クライアント） |
