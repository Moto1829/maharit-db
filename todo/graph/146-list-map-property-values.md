# タスク 146: プロパティの値としてリスト（とマップ）を保存できるようにする

## 現状（2026-11-08 確認）
- `CREATE (:P {tags: ['a', 'b']})` が `type error: cannot convert to property value` になる（`executor.rs` の `value_to_property`）
- `PropertyValue` に List / Map がない。パラメータでリストを渡しても同じエラー
- Cypher（Neo4j）ではプリミティブ型の同種リストをプロパティにできる

## 作業
- [ ] `PropertyValue::List(Vec<PropertyValue>)` を追加（Neo4j に合わせて同種のプリミティブのみ許すか検討）。Map は Neo4j でも不可なので対象外でよい
- [ ] 永続化フォーマット（`snapshot_codec.rs`）に新しいタグを追加し `VERSION` を上げる。旧版の読み込みは維持（CLAUDE.md の永続化ルール）
- [ ] WAL / レプリケーション（`WalEntryData::SetProperty` の JSON 表現）、バックアップ、`maharit-io` の入出力、インデックス（リストはインデックス対象外にするか）
- [ ] `IN`、`size()`、インデックスアクセスなどがプロパティのリストに効くことを確認
- [ ] docs/cypher/parameters.md の注意事項を更新
