# server/92: フォロワーに --node-id フラグを追加（node_id 衝突の解消）

## 背景
e2e（replication_test）実施時に判明: `main.rs` の CLI には node_id を設定する
フラグが無く、`ReplicationConfig` の既定 `node_id = "node-1"` が全ノードで共有される。
`start_replication_local.sh` で 2 フォロワーを起動すると両方が `follower_id = "node-1"`
となり、リーダーの `followers: HashMap<String, FollowerState>`（follower_id キー）で
衝突する。起動直後の初回接続で片方が部分同期する一過性の不安定さの一因（再起動で解消し、
決定論バグではないが運用上のギャップ）。

## やること
- `main.rs` に `--node-id <ID>` フラグ（+ 環境変数 `MAHARIT_NODE_ID`）を追加し、
  `ReplicationConfig.node_id` に反映（leader/follower 両方）。未指定時は既定 `node-1`。
- `start_replication_local.sh` で各ノードに個別 node_id を付与
  （例: leader / follower1 / follower2）。
- ヘルプテキスト更新。可能なら「フォロワー同一 node_id を検知して警告」も検討。

## 受け入れ条件
- ローカル 3 ノードで各ノードが一意の node_id を持ち、初回接続からフォロワー 2 台が
  安定して全同期する。
- ヘルプに `--node-id` が表示される。

## 優先度 / 規模
- 中（運用ギャップ解消）。小規模・低リスク（CLI 配線 + スクリプト）。

## 対応（完了）
- `main.rs` に `--node-id <ID>` フラグ + 環境変数 `MAHARIT_NODE_ID` を追加。
  解決（フラグ > env > 既定 `node-1`）した値を leader/follower の
  `ReplicationConfig.node_id` に反映。ヘルプテキスト更新。
- `start_replication_local.sh` で各ノードに個別 node-id を付与
  （`leader` / `follower1` / `follower2`）。

## 検証
- ローカル 3 ノードで各ノードが一意の node_id を持つ。
- **クラスター再起動直後の初回 replication_test で 26/26 通過**
  （従来はフォロワー2が初回接続時に部分同期する一過性の揺れがあったが、
   node_id 衝突解消により初回から安定して全同期）。
- failover_test --no-docker: 18/18 通過。server テスト 240 件パス。

## ステータス
完了
