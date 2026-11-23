# タスク 217: レプリケーションの状態を返すプロシージャ（db.replication.status）

## 背景（2026-11-23、task127 の残り）
task127（task148）で docs から外したプロシージャ。レプリケーションの状態は `maharit-server/src/replication.rs`
（`LeaderReplicationManager` の `get_follower_count` など）にあるが、Cypher から見る手段は無い。

※ 実装するかどうかは後で決める。決めるまでは着手しない。

## 候補
- `CALL db.replication.status() YIELD role, leader, followers, lastAppliedOffset, lagBytes`
  （リーダーではフォロワーごとに 1 行: `address, connectionId, alive, lastHeartbeat, lag` を返す案も）

## 課題
- サーバー側の状態に触るので、task215 のサーバー側プロシージャの仕組みが要る
- 返す列を、フォロワー・リーダーのどちらで呼んでも意味が通るように決める
- 権限（reader でよいか）、HTTP の `/health/ready` や `/metrics` との重複
- Bolt を採る場合、ドライバのルーティング（`ROUTE`）の情報と整合させる

## 作業
- [ ] 列を決め、リーダー・フォロワーそれぞれで返す
- [ ] レプリケーションの E2E（`scripts/replication_test.py`）に確認を追加
- [ ] docs/advanced のレプリケーションの章（日英）に追加
