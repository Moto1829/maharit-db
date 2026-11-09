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
  ├── 接続時にスナップショットを受信して置き換え
  ├── 以降の WAL を受信・適用
  └── 読み取りのみ（書き込みは拒否）
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
| `--replication-bind` | 両方 | レプリケーションの待ち受けアドレス（デフォルト `127.0.0.1:7688`）。フォロワーでは、昇格した後に使う |
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

接続が切れると、フォロワーは 1 秒ごとにリーダー（`--leader-addr`）へ接続し直します。リーダーより先にフォロワーを起動しても構いません。

## フォロワーからの読み取り

フォロワーは読み取りクエリだけを受け付けます。書き込みクエリには `this node is a read-only follower: send writes to the leader` のエラーを返すので、書き込みはリーダーに送ってください。

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

フォロワーがリーダーに接続する（接続し直す）たびに、リーダーはその時点のグラフ全体をスナップショットとして送り、続けてそれ以降の変更を送ります。スナップショットは書き込みを一瞬止めて取るので、スナップショットとその後の変更の間に抜けや重なりはありません。

フォロワーはスナップショットを受け取ると、自分のグラフを丸ごと置き換えます。フォロワー自身のデータファイルの内容や、切断中の古いデータは残りません。データのあるリーダーに、空のフォロワーをそのまま追加できます。

- 置き換えの間（大きなグラフでは数秒）、そのフォロワーへの読み取りは途中の状態を返すことがあります。
- インデックス・制約・全文検索インデックスの定義もスナップショットと一緒に届き、以降の `CREATE` / `DROP` も複製されます。フォロワーはそれぞれのインデックスを自分で作り、複製された変更に合わせて更新します。フォロワーで直接 `CREATE INDEX` などを実行することはできません（書き込みとして拒否されます）。

## フェイルオーバー

リーダーが停止したら、フォロワーの 1 台を昇格させます。`--addr` は、そのフォロワーの**クライアント用**アドレスです。

```bash
maharit admin promote-to-leader --addr follower-1:7687
# TLS のフォロワー:        --ca /etc/maharit/ca.crt
# --require-auth のフォロワー: --password <admin のパスワード>（または MAHARIT_ADMIN_PASSWORD、--user で別の管理者）
```

昇格したフォロワーは、次の順に切り替わります。

1. 旧リーダーからの受信をやめる
2. その時点のグラフをデータファイルに保存し、WAL を使い始める（以降の書き込みはクラッシュしても失われない）
3. 自分の `--replication-bind`（デフォルト `127.0.0.1:7688`）でフォロワーの接続を受け付け始める
4. 書き込みを受け付ける

残りのフォロワーは、`--leader-addr` を新しいリーダーの `--replication-bind` に向けて起動し直すと、スナップショットで追いつきます。あらかじめ全フォロワーの `--leader-addr` を、昇格後のリーダーに届く仮想 IP や DNS 名にしておけば、起動し直す必要もありません（フォロワーは自動で接続し直します）。

- 昇格の指示は管理者（`--require-auth` のときは admin ロール）だけが送れます。2 回目の昇格はエラーになります。
- 昇格したノードを再起動すると、起動時のフラグどおりフォロワーに戻ります。リーダーとして使い続けるときは、`--replication-role leader` に変えて起動してください。
- 旧リーダーを戻すときは、フォロワーとして（`--replication-role follower --leader-addr <新しいリーダー>`）起動します。旧リーダーのデータは、新しいリーダーのスナップショットで置き換わります。旧リーダーをリーダーのまま起動すると、リーダーが 2 台になり、書き込みが分かれます。
- 自動フェイルオーバー（リーダー停止の検知と昇格の自動実行）には対応していません。`stats` の `is_leader_alive` を監視して、昇格を実行してください。

## レプリケーション設定のまとめ

| 設定項目 | リーダー | フォロワー |
|---------|--------|---------|
| 書き込み | 対応 | 拒否（昇格すると受け付ける） |
| 読み取り | 対応 | 対応（ラグあり） |
| ハートビート送信 | 送信 | 受信 |
| WAL ストリーミング | 送信 | 受信・適用 |
| 永続化 | データファイル + WAL | 停止時にデータファイルを保存（接続時にリーダーのスナップショットで置き換え）。昇格後は WAL も使う |
| ポート | 7687（クライアント）、7688（レプリケーション） | 7687（クライアント）。昇格後は 7688 も |
