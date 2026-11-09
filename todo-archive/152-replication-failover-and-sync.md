# バグ 152: 昇格コマンドが機能せず、フォロワーが書き込みを受け付け、既存データが同期されない

## 現状（2026-11-08、server/150 の作業中に確認）
1. **`maharit admin promote-to-leader --addr <ADDR>` が何もしない**: `send_promote_to_leader` は `ReplicationMessage::PromoteToLeader` をフォロワーの**クライアント用ポート**（TcpServer）に送って終了コード 0 を返す。TcpServer はこれを解釈できない。さらに `FollowerReplicationManager::is_promoted()` はどこからも参照されておらず、受信できても昇格しない
2. **フォロワーが書き込みクエリを受け付ける**: TcpServer はフォロワーでも書き込みを実行するため、リーダーとデータがずれる。`scripts/failover_test.py` のフェーズ 5（昇格後の書き込み）は、この挙動のおかげで通っている
3. **既存データのスナップショット同期が無い**: `main.rs` はリーダーを `LeaderReplicationManager::new`（graph = None）で作るため、新しいフォロワーに既存データを送らない。`with_graph` は `Arc<RwLock<Graph>>` を取り、サーバーの `ConcurrentGraph` と型が合わない
- docs（advanced/replication.md）には 3 点とも制限として記載済み

## 作業
- [x] フォロワーは書き込みクエリをエラーにする（`this node is a read-only follower`、昇格後は受け付ける）
- [x] 昇格: クライアントポートの `promote` リクエスト（admin のみ）。旧リーダーからの受信停止 → チェックポイント（データファイル保存 + WAL 開始）→ `--replication-bind` でレプリケーション待ち受け → 書き込み許可。`admin promote-to-leader` は `--ca` / `--user` / `--password` 対応で作り直し。旧 `PromoteToLeader` メッセージは無視
- [x] スナップショット同期: `SnapshotSource`（サーバーのグラフ + write gate）。gate を持ったまま購読登録とスナップショット取得を行い、抜け・重複なし（ゲートを外すとテストが失敗することを確認）。フォロワーは受信時にグラフを `clear()` して置き換え
- [x] フォロワーの自動再接続（1 秒間隔、接続ごとにスナップショット）。リーダーより先に起動しても待つ
- [x] フォロワーも WAL を開く（昇格後の耐久性のため）
- [x] failover_test: フォロワーの書き込み拒否、実際の昇格、フォロワー2 の新リーダーへの自動再接続を検証。tls_test: TLS 経由・認証付きの昇格
- [x] docs（advanced/replication.md、api/protocol.md、server-config.md）

## 残した制限
- インデックス・制約は複製されない（storage/149 と同根）
- スナップショット適用中のフォロワーの読み取りは途中の状態を返しうる
- フォロワーでは `CREATE INDEX` / 制約の作成も書き込みとして拒否されるため、フォロワーにはインデックス・制約が無い（スキーマの複製は storage/149 に追記）
