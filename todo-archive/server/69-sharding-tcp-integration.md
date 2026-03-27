# Task 69: シャーディングを TcpServer に統合する

## 背景・目的

`maharit-cluster` クレートはシャーディングのルーティング・コーディネーターをライブラリとして実装済みだが、
TcpServer との統合がなく、実際のネットワーク越しのクエリ転送は未実装（in-process シミュレーションのみ）。

本タスクでは TcpServer をシャードノードとして動作させ、コーディネーターが各シャードに
クエリを転送・結果をマージする実際のクラスターを構築できるようにする。

## 現状

- `maharit-cluster`: `ClusterCoordinator`, `QueryRouter`, `ShardingStrategy` 実装済み
- `maharit-server/src/replication.rs`: TCP 通信・メッセージフレーミング実装済み
- **未実装**: コーディネーターから各シャードへのクエリ転送（TCP）
- **未実装**: TcpServer のシャードモード起動
- **未実装**: クエリ結果の収集・マージ
- **未実装**: CLI フラグ・設定ファイル対応

## 実装計画

### Phase 1: シャードクライアント実装

コーディネーターが各シャードノードに TCP でクエリを送るクライアントを実装する。

- `maharit-cluster/src/shard_client.rs` を新規作成
  - `ShardClient`: 単一シャードへの接続を管理（`maharit-client` の `Client` を流用可）
  - `execute(query: &str) -> Vec<Row>`: クエリ送信・結果受信
  - 接続プール（再接続・タイムアウト）
- `ClusterCoordinator::execute_all(query)` の実ネットワーク実装
  - 現在は in-process のみ → 各 `ShardClient` にクエリを送信
  - 結果を `merge_results()` でマージして返す

### Phase 2: TcpServer のシャードモード

TcpServer がシャードノードとして動作するモードを追加する。

- `main.rs` に `--shard` フラグ追加
  - `--shard-id <id>`: このノードのシャード ID
  - `--coordinator-addr <addr>`: コーディネーターのアドレス（登録用）
- シャードノードは通常のクエリ受信に加え、自分の担当範囲のデータのみ保持
- ノード作成時にシャーディング戦略に従ったルーティングチェック（担当外 ID は拒否またはリダイレクト）

### Phase 3: コーディネーターノード

クエリを受け取り各シャードに転送する専用の軽量コーディネーターを実装する。

- `main.rs` に `--coordinator` フラグ追加
  - `--coordinator` が指定されると TcpServer ではなく `ShardCoordinatorServer` を起動
  - クライアントから Cypher クエリを受信
  - `ClusterCoordinator` でルーティング判定
  - 対象シャードにクエリ転送、結果収集・マージ
  - マージ結果をクライアントに返却
- 設定ファイル（TOML）対応
  ```toml
  [sharding]
  enabled = true
  strategy = "hash"
  replication_factor = 1

  [[sharding.shards]]
  id = 0
  address = "node0:7687"

  [[sharding.shards]]
  id = 1
  address = "node1:7687"
  ```

### Phase 4: クロスシャードクエリ対応

- `MATCH (a)-[r]->(b)` でノードが別シャードにある場合の処理
- 全シャードに問い合わせて結果をマージする `AllShards` ルーティング
- クロスシャードエッジの整合性維持（`EdgeLocation::Remote`）

## 起動例（完成後）

```bash
# シャード 0
maharit server --port 7687 --shard --shard-id 0

# シャード 1
maharit server --port 7688 --shard --shard-id 1

# コーディネーター（クライアントはここに接続）
maharit server --port 7690 --coordinator \
  --shards "0:localhost:7687,1:localhost:7688" \
  --strategy hash
```

## 注意事項

- Phase 1 → Phase 2 → Phase 3 の順で実装する
- 各 Phase でビルド・既存テストが通ることを確認する
- 分散トランザクション（2PC）は本タスクのスコープ外
- 自動シャード再バランスは本タスクのスコープ外

## ステータス

- [x] Phase 1: ShardClient 実装（TCP クエリ転送・結果受信）
- [x] Phase 2: TcpServer シャードモード（`--shard` フラグ）
- [x] Phase 3: コーディネーターノード（`--coordinator` フラグ、TOML 設定）
- [x] Phase 4: クロスシャードクエリ対応（全シャードへのブロードキャスト + dedup マージ）
