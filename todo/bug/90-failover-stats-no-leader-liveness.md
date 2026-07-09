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

## あるべき対応（要判断）
- `stats`（または新規 `replicationStats` リクエスト）でリーダー生存/LSN/フォロワー数を公開する。
- もしくは failover_test を、実際にリーダープロセスを停止し
  `admin promote-to-leader` を発行する自動フロー（--no-docker でも）に改める。

## ステータス
未対応（原因切り分け済み・既存問題として記録）。中核のフェイルオーバー機能自体は動作。
