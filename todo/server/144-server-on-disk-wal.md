# タスク 144: サーバーがディスク上の WAL を書かず、強制終了で変更が失われる

## 現状
- `maharit server` はデータファイルを起動時に読み込み、SIGINT / SIGTERM を受けたときだけ `PersistentStorage::save_concurrent` で保存する（`crates/maharit-server/src/main.rs` の `spawn_shutdown_handler`）。
- 定期保存もディスク上の WAL もないため、`kill -9`・OOM・電源断では、起動以降の変更がすべて失われる。
- WAL（`maharit-storage` の `wal.rs` / `wal_group_commit.rs`）はライブラリとしてはあるが、サーバーでは使われていない。サーバーの「WAL」はレプリケーション用のメモリ上のエントリ（`mutation_log.rs`）。
- README・docs は「WAL でクラッシュ後もリカバリ」と書いていたので、2026-11-07 に実装に合わせて書き換えた（index.md / architecture.md / README）。

## 作業
- [ ] 書き込みクエリの `WalEntryData` をディスク上の WAL に追記（グループコミットで fsync をまとめる）
- [ ] 起動時: データファイルを読み込んだあと WAL を再適用
- [ ] チェックポイント: 定期的に（または WAL サイズで）スナップショットを保存し WAL を切り詰める
- [ ] `kill -9` からの復旧を確かめる E2E（`scripts/persistence_test.py` の拡張）
- [ ] durability モードのフラグ（毎コミット fsync / グループコミット / なし）を検討
- [ ] 完了後、docs と README の warning を外す
