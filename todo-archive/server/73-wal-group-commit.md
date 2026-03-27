# Task 73: WAL グループコミット（非同期バッファリング）

## 概要
現在の WAL は1書き込みごとに同期フラッシュしていると考えられる。
グループコミット（複数の書き込みをまとめて1回フラッシュ）を導入し、
書き込みレイテンシを RTT 程度（< 1 ms）まで削減する。

## 背景（ベンチマーク根拠）
- CREATE nodes: 7 ms/op — RTT（< 1 ms）に対して大きすぎる
- WAL の同期フラッシュがボトルネックと推定
- 耐久性とのトレードオフが発生するため設定で制御可能にする

## 実装内容

### グループコミット実装
- [x] 書き込みリクエストを一定時間（デフォルト: 5ms）または一定件数（デフォルト: 100件）バッファリング
- [x] バッファを定期的にまとめて fsync する非同期タスクを tokio で実装（`tokio::select!` + `time::interval`）
- [x] 各リクエストは自分の WAL エントリが flush されるまで await する（oneshot channel）

### 設定項目
- [x] `flush_interval_ms`（デフォルト: 5ms）
- [x] `flush_batch_size`（デフォルト: 100件）
- [x] `WalGroupCommitConfig::synchronous()` で同期モード（0ms / batchsize=1）に切り替え可能

### 耐久性の保証
- [x] グループコミット時の障害リカバリをテスト（WAL 再オープン＋`recover()` で確認）
- [x] `COMMIT` レスポンスは flush 完了後に返すことを保証（oneshot で LSN 通知）

## 実装ファイル
- `crates/maharit-storage/src/wal_group_commit.rs` — `WalGroupCommitter`, `WalGroupCommitConfig`
- `crates/maharit-storage/src/lib.rs` — pub re-export

## テスト（5件）
- `test_group_committer_basic` — 基本書き込み・LSN 検証
- `test_group_committer_synchronous_mode` — 同期モード動作確認
- `test_group_committer_batch_100_writes` — 100 件並列書き込みの LSN 一意性
- `test_group_committer_durability` — fsync 後に WAL を再オープンしてリカバリ検証
- `test_group_committer_config_custom` — カスタム設定

## ステータス
完了。maharit-storage: 67 tests passing。バージョン 0.2.0 に更新。
