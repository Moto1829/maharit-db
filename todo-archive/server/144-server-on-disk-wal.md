# タスク 144: サーバーがディスク上の WAL を書かず、強制終了で変更が失われる

## 現状
- `maharit server` はデータファイルを起動時に読み込み、SIGINT / SIGTERM を受けたときだけ `PersistentStorage::save_concurrent` で保存する（`crates/maharit-server/src/main.rs` の `spawn_shutdown_handler`）。
- 定期保存もディスク上の WAL もないため、`kill -9`・OOM・電源断では、起動以降の変更がすべて失われる。
- WAL（`maharit-storage` の `wal.rs` / `wal_group_commit.rs`）はライブラリとしてはあるが、サーバーでは使われていない。サーバーの「WAL」はレプリケーション用のメモリ上のエントリ（`mutation_log.rs`）。
- README・docs は「WAL でクラッシュ後もリカバリ」と書いていたので、2026-11-07 に実装に合わせて書き換えた（index.md / architecture.md / README）。

## 作業
- [x] 書き込みクエリの `WalEntryData` をディスク上の WAL に追記（グループコミットで fsync をまとめる）
- [x] 起動時: データファイルを読み込んだあと WAL を再適用
- [x] チェックポイント: 定期的に（または WAL サイズで）スナップショットを保存し WAL を切り詰める
- [x] `kill -9` からの復旧を確かめる E2E（`scripts/persistence_test.py` の拡張）
- [x] durability モードのフラグ（`--wal fsync|os|off`、`--checkpoint-mb`）
- [x] 完了後、docs と README の warning を外す

## 結果（2026-11-08）
- 新モジュール `crates/maharit-server/src/durable_wal.rs`: `<data>.wal` に書き込みクエリ単位で `WalEntryData` を追記（長さ + CRC32 + JSON）。ヘッダーにスナップショットの SHA-256 を持ち、チェックポイントが途中で落ちた場合の古い WAL は捨てる（二重適用しない）。書きかけの末尾レコードは切り捨て
- グループコミット: 書き込みゲート内で追記し、ゲートを出てから `sync_to` で fsync を待つ。並行書き込みは 1 回の fsync を共有
- `--wal fsync|os|off`（既定 fsync、`MAHARIT_WAL`）、`--checkpoint-mb`（既定 64、0 で停止時のみ）。フォロワーは WAL を使わない
- チェックポイント（停止時・サイズ超過時）: 書き込みゲートを取ってスナップショット保存 → WAL を空にする
- スナップショット保存を原子的に（tmp + fsync + rename + ディレクトリ fsync）。以前はその場で上書きしており、保存中に落ちるとファイルが壊れた
- `load_concurrent` がノード・エッジの ID を振り直していたのを、保存時の ID を保つよう変更（稼働中のチェックポイント後の WAL が ID で正しく適用されるため）
- WAL / レプリケーションの `SetProperty` で date / datetime / duration が文字列に化けていたのを、型付きで往復するよう修正（レプリケーションのフォロワーにも効く）
- リクエストのトレーシングスパンを `span.enter()` のまま `.await` をまたいでいたため、並行書き込みで tracing-subscriber が panic することがあった（`--wal os` のベンチで再現）。`.instrument(span)` に修正
- テスト: durable_wal の単体テスト 7 件、稼働中チェックポイント + 疎な ID + date + ロールバックを含むクラッシュ復旧の一致テスト、ID 保持の保存テスト、`scripts/persistence_test.py` に SIGKILL 直前の書き込みが復元される検査を追加（旧バイナリでは失敗することを確認）
- 性能（macOS、release、Python クライアント）: off 約 12,500 / os 約 10,700 / fsync 約 320 逐次書き込み/秒。8 並列では fsync 約 920/秒（グループコミットで約 3 倍）
- 残り: TLS 未組み込み（server/147）、運用ドキュメントの照合（docs/148）、スキーマの永続化（storage/149）。未コミットのトランザクションの変更はクラッシュ後に残る（docs に制限として記載）
