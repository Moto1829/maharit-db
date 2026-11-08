# バグ 152: 昇格コマンドが機能せず、フォロワーが書き込みを受け付け、既存データが同期されない

## 現状（2026-11-08、server/150 の作業中に確認）
1. **`maharit admin promote-to-leader --addr <ADDR>` が何もしない**: `send_promote_to_leader` は `ReplicationMessage::PromoteToLeader` をフォロワーの**クライアント用ポート**（TcpServer）に送って終了コード 0 を返す。TcpServer はこれを解釈できない。さらに `FollowerReplicationManager::is_promoted()` はどこからも参照されておらず、受信できても昇格しない
2. **フォロワーが書き込みクエリを受け付ける**: TcpServer はフォロワーでも書き込みを実行するため、リーダーとデータがずれる。`scripts/failover_test.py` のフェーズ 5（昇格後の書き込み）は、この挙動のおかげで通っている
3. **既存データのスナップショット同期が無い**: `main.rs` はリーダーを `LeaderReplicationManager::new`（graph = None）で作るため、新しいフォロワーに既存データを送らない。`with_graph` は `Arc<RwLock<Graph>>` を取り、サーバーの `ConcurrentGraph` と型が合わない
- docs（advanced/replication.md）には 3 点とも制限として記載済み

## 作業
- [ ] フォロワーは書き込みクエリをエラーにする（昇格後は受け付ける）
- [ ] 昇格: 管理用の経路（クライアントポートの管理者専用リクエスト等、認証必須）で `is_promoted` を立て、リーダーとして動作を切り替える（WAL・レプリケーション待ち受けの開始）
- [ ] `LeaderReplicationManager` が `ConcurrentGraph` からスナップショットを作れるようにし、`main.rs` で渡す（スナップショットと WAL ストリームの境界の LSN を揃える）
- [ ] failover_test を実際の昇格を検証する形に直す。docs の制限を外す
