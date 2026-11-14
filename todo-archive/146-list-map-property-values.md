# タスク 146: プロパティの値としてリスト（とマップ）を保存できるようにする

## 現状（2026-11-08 確認）
- `CREATE (:P {tags: ['a', 'b']})` が `type error: cannot convert to property value` になる（`executor.rs` の `value_to_property`）
- `PropertyValue` に List / Map がない。パラメータでリストを渡しても同じエラー
- Cypher（Neo4j）ではプリミティブ型の同種リストをプロパティにできる

## 作業
- [x] `PropertyValue::List(Vec<PropertyValue>)` を追加（Neo4j に合わせて同種のプリミティブのみ許すか検討）。Map は Neo4j でも不可なので対象外でよい
- [x] 永続化フォーマット（`snapshot_codec.rs`）に新しいタグを追加し `VERSION` を上げる。旧版の読み込みは維持（CLAUDE.md の永続化ルール）
- [x] WAL / レプリケーション（`WalEntryData::SetProperty` の JSON 表現）、バックアップ、`maharit-io` の入出力、インデックス（リストはインデックス対象外にするか）
- [x] `IN`、`size()`、インデックスアクセスなどがプロパティのリストに効くことを確認
- [x] docs/cypher/parameters.md の注意事項を更新

## 結果（2026-11-14）
- `PropertyValue::List(Vec<PropertyValue>)` を追加。要素はリスト以外の値（null・混在した型は許す）。リストのリスト・マップはエラー
- スナップショット: リストを含むときだけ新しい v6（v5 と同じ並び、値のタグ 8 = リスト）で書く。リストが無ければ従来どおり v4 / v5 なので旧版でも読める。v4 フィクスチャのテストはそのまま通る
- ストレージ WAL・バックアップ: 値のタグ 8 を追加（ヘッダーのバージョンは据え置き。リストを含む WAL・バックアップは旧版では読めない）。ネストしたリストは破損扱い
- レプリケーション・サーバーの WAL（`property_value_to_wal_string`）: JSON 配列で書き、要素の型（1.0 の Float、日付など）も戻る
- maharit-io: JSON の配列（要素に配列・オブジェクトが無いもの）をリストとして読み込み、リストを配列で書き出す。CSV / GraphML / DOT は文字列で出す
- クエリ: IN・size()・添字・UNWIND・`n.tags + [...]`・パターン `{tags: [...]}`・`=` での比較が効く。プロパティインデックスは等号の検索に使える（範囲検索は対象外）
- サーバーで kill -9 後の WAL 再生・正常終了後の v6 スナップショットからの再起動で値と型が戻ることを確認
- docs: parameters.md・basics.md（日英）を更新。unwind-with-union.md の最初の例（`UNWIND a.tags`）が動くようになった
- 気づいたこと（未対応）: `SET n.p = null` でプロパティが消えずに null が残る（Neo4j では削除）。Python ネイティブ拡張（clients/python-native）はリストを文字列で返す
