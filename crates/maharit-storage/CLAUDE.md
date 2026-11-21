# maharit-storage

スナップショット（ファイル全体の保存・復元）、WAL、MVCC、バックアップ。

## ファイル構成

- `persistence.rs`: `PersistentStorage`。保存は tmp に書いて fsync → rename（原子的）
- `snapshot_codec.rs`: v4 以降のバイト列（bincode 1 互換）を自前で読み書きする。bincode には依存しない
- `wal.rs` / `wal_group_commit.rs`: WAL とグループコミット（サーバーのディスク WAL は `maharit-server` の `durable_wal.rs`）
- `mvcc.rs` / `transaction.rs`: MVCC トランザクション。`backup.rs`: `Backup::create` / `restore` / `verify`（None / Gzip / Zstd）

## スナップショットの形式

| 版 | 書く関数 | 中身 |
|---|---|---|
| v4 | `save` / `save_with_index` | グラフ + プロパティ索引 |
| v5 | `save_concurrent_with_schema`（サーバー） | v4 の後ろにスキーマ（索引・制約の定義、JSON 文字列） |
| v6 | 同上 | v5 と同じ並びで、プロパティ値にリストを含むとき。リストが無ければ v4 / v5 で書くので旧版でも読める |

v1〜v3 は読み込みだけ。

## 形式を変えるとき

1. 新しい版の番号を足す（`VERSION_V<n>`）。既存の版の書き方は変えない
2. 旧版の読み込みを残す。読めなくなる変更は互換を壊す（`feat!:`）
3. 旧版の書き方で作ったファイルを `tests/fixtures/` に置き、読めることをテストする（`v4_*.mgdb` は bincode 時代に書いた互換確認用で、作り直さない）
4. 新しい型を足すときは、使わない限り旧版で書く（v6 のリストと同じ）。旧バージョンのサーバーで読めるデータを増やさない
5. WAL（`WalEntryData`）の形を変えるときも同じ。レプリケーションで新旧のサーバーが混ざる

## 知らないと間違えること

- `load_concurrent` はノード・エッジの ID を保存時のまま復元する（WAL が ID で対象を指すため）
- スキーマの中身（索引・制約の定義）は `maharit-server` の `schema.rs` が決める。このクレートは文字列として運ぶだけ
