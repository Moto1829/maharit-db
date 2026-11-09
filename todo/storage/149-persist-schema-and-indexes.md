# タスク 149: インデックス・制約・全文検索インデックスの定義を永続化する

## 現状（2026-11-08 確認）
- サーバーのスナップショット保存（`PersistentStorage::save_concurrent`）は `indexes: vec![]` で、プロパティインデックス・UNIQUE 等の制約・全文検索インデックスの定義を保存しない
- WAL（`WalEntryData`）もグラフの変更だけを記録し、`CREATE INDEX` / `CREATE CONSTRAINT` / `CREATE FULLTEXT INDEX` は記録しない
- そのため再起動すると定義が消え、制約が効かなくなる（`docs/operations/server-config.md` の「制限」に記載）

## 作業
- [ ] スナップショットにスキーマ（インデックス・制約・全文検索インデックスの定義）を保存し、読み込み時にインデックスを再構築
- [ ] DDL を WAL に記録（`WalEntryData` にスキーマ変更の variant を追加）し、再生とレプリケーションで適用
- [ ] 再起動後に UNIQUE 制約が効くことの E2E（`scripts/constraint_test.py` の拡張）

## 追記（2026-11-09、bug/152）
- フォロワーは書き込みを拒否するようになり、`CREATE INDEX` / 制約の作成もできない。スキーマ定義を保存する際に、レプリケーション（スナップショットと WAL）でもフォロワーに送り、フォロワーでインデックスを構築する
