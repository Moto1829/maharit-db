# Task 80: レプリケーション フェイルオーバーテスト

## 背景・目的

`replication.rs` には `PromoteToLeader` メッセージと
`maharit admin promote-to-leader` コマンドが実装されているが、
フェイルオーバーの一連フローを検証するテストがない。

## 実装内容

### Python スクリプト: `scripts/failover_test.py`

対象環境: `docker-compose.replication.yml`（リーダー1 + フォロワー2）

#### テストフロー

```
1. 初期状態確認
   - リーダー・フォロワー2台に接続
   - リーダーにデータを書き込む

2. リーダー停止
   - docker stop maharit-leader

3. フォロワー1を昇格
   - maharit admin promote-to-leader --addr localhost:7689（レプリケーションポート）

4. 昇格後の動作確認
   - 旧フォロワー1（新リーダー）に書き込みができること
   - 旧フォロワー1で MATCH が返ること（既存データが保持されていること）

5. フォロワー2の動作確認
   - フォロワー2が新リーダーに追従できること（将来実装）
   - または: フォロワー2の is_leader_alive が false になること

6. 旧リーダー再起動（オプション）
   - 旧リーダーがフォロワーとして新リーダーに接続できること
```

#### 検証項目

- 昇格後の新リーダーに書き込みができること
- 昇格前にレプリケートされたデータが失われないこと
- `is_leader_alive` フラグが正しく更新されること
- ハートビートタイムアウト検知が機能すること

### Rust テスト

```rust
#[tokio::test]
async fn test_promote_to_leader() {
    // フォロワーに PromoteToLeader メッセージを送信
    // → is_promoted() が true になること
}

#[tokio::test]
async fn test_heartbeat_timeout_detection() {
    // リーダーとの接続を切断
    // heartbeat_timeout_secs 経過後に is_leader_alive が false になること
}
```

## 前提条件

- Task 77（CI パイプライン）完了後に CI へ追加
- フォロワーが新リーダーへ再接続する機能は別タスクで対応

## 完了条件

- [ ] リーダー停止後にフォロワーを昇格できること
- [ ] 昇格後の新リーダーへの書き込みが成功すること
- [ ] ハートビートタイムアウト検知のテストがあること
- [ ] `python3 scripts/failover_test.py` で実行可能
