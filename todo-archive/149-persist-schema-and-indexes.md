# タスク 149: インデックス・制約・全文検索インデックスの定義を永続化する

## 現状（2026-11-08 確認）
- サーバーのスナップショット保存（`PersistentStorage::save_concurrent`）は `indexes: vec![]` で、プロパティインデックス・UNIQUE 等の制約・全文検索インデックスの定義を保存しない
- WAL（`WalEntryData`）もグラフの変更だけを記録し、`CREATE INDEX` / `CREATE CONSTRAINT` / `CREATE FULLTEXT INDEX` は記録しない
- そのため再起動すると定義が消え、制約が効かなくなる（`docs/operations/server-config.md` の「制限」に記載）

## 作業
- [x] スナップショットにスキーマを保存（データファイル v5 = v4 ペイロード + スキーマ JSON 文字列。v4 も読める）し、起動時にインデックスを再構築（`schema.rs` の `Schema::install`）
- [x] DDL 成功時に `WalEntryData::SetSchema`（スキーマ全体、置き換え方式）を WAL とフォロワーに送る。再生では最後の SetSchema が有効
- [x] E2E: `persistence_test.py` にスキーマの再起動テスト（WAL あり SIGTERM/SIGKILL、`--wal off`、backup → restore）。`failover_test.py` で複製された UNIQUE 制約が昇格後に効くこと

## 追記（2026-11-09、bug/152）
- フォロワーは書き込みを拒否するようになり、`CREATE INDEX` / 制約の作成もできない。スキーマ定義を保存する際に、レプリケーション（スナップショットと WAL）でもフォロワーに送り、フォロワーでインデックスを構築する

## 実施メモ（2026-11-09）
- フォロワー: スナップショットの末尾にスキーマを付けて送り、受信側で install。以降の DDL も SetSchema で届く。複製された変更でプロパティ索引・全文索引を更新するようにした（これまでフォロワーの索引は古いままだった。テストで確認）
- `maharit backup` / `restore` もスキーマを運ぶ（バックアップ v2: ヘッダーの description の後にスキーマ文字列。v1 も読める）
- `--wal off` のシャットダウン保存もスキーマを含める（`SnapshotWriter`）
- ロールバック時の索引更新は `schema::reindex_nodes` に統一（主ラベルだけでなく全ラベルで判定）
- 制限: トランザクション内の DDL はロールバックで取り消されない（従来どおり、docs に記載）
