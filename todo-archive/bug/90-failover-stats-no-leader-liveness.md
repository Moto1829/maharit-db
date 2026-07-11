# bug/90: failover_test の残1件 — stats が is_leader_alive を公開せず、非対話ローカルモードで昇格が自動化されない

## 概要
`failover_test.py --no-docker` は 16 件中 15 件通過。フェイルオーバーの中核
（フォロワー昇格 `admin promote-to-leader`、昇格後のデータ保持、新リーダーへの書き込み/読み取り）は
すべて通過する。残る 1 件のみ失敗:

```
✗ フォロワー2: is_leader_alive が false になっている
  stats={'type':'stats','connections':1,'total_queries':8,'nodes':3,'edges':1}
```

## 原因（2 点、いずれも既存・本セッションの改修とは無関係）
1. **`stats` コマンドがレプリケーション生存情報を公開していない。**
   TCP の `stats` レスポンスは `connections/total_queries/nodes/edges` のみで、
   `is_leader_alive`（`ReplicationStats` が内部的に持つ）を含まない。テストはこれを
   期待しているが取得できない。
2. **`--no-docker` 非対話モードでは実際のリーダー停止/昇格が手動前提。**
   テストは「手動で kill / promote してください」と表示して自動続行するため、
   リーダーが実際には停止されず、フォロワー2 がハートビートタイムアウトで
   `is_leader_alive=false` に遷移する条件が発生しない。

## 対応（完了）— 両方実施
1. **プロトコル拡張**（`tcp_server.rs`）:
   - `ReplicationStatus { role, is_leader_alive }` を追加し、`stats` レスポンスに
     `replication` フィールド（standalone では省略）を追加。
   - `TcpServer` に `follower: Option<Arc<FollowerReplicationManager>>` と
     `with_follower()` を追加。`stats` ハンドラで follower なら
     `is_leader_alive()`、leader なら常に true を返す。
   - `main.rs` のフォロワー分岐で `.with_follower(...)` を配線。
2. **テストフロー自動化**（`failover_test.py`）:
   - `--no-docker` 非対話モードで、PID ファイル先頭のリーダーを実際に SIGKILL
     （`kill_local_leader`）、検出したバイナリで `admin promote-to-leader` を実行。
   - `admin promote-to-leader --addr <follower>` はフォロワーに直接接続して昇格させ、
     リーダー不要（`send_promote_to_leader`）。

## 検証
- `failover_test.py --no-docker`（ローカル 3 ノード）: **18/18 通過**。
  リーダー SIGKILL → フォロワー1昇格 → フォロワー2 が `stats.replication.is_leader_alive`
  でリーダー死亡（ハートビートタイムアウト）を検出。

## ステータス
完了
