# Task 78: 永続化ラウンドトリップテスト

## 背景・目的

サーバーのシャットダウン → 再起動後にデータが正しく復元されることを
E2E レベルで検証するテストがない。

`PersistentStorage` の単体テストは存在するが、
「サーバーを起動してクエリで書き込み → 停止 → 再起動 → クエリで読み取り」
という実際の運用フローのテストはない。

## 実装内容

### Python スクリプト: `scripts/persistence_test.py`

#### テストフロー

```
1. サーバー起動（--data /tmp/test_maharit.db）
2. CREATE でノード・エッジ・プロパティを書き込む
3. SIGTERM でサーバーを正常終了（保存をトリガー）
4. 同じデータファイルでサーバーを再起動
5. MATCH でデータが残っていることを確認
6. クリーンアップ
```

#### 検証項目

- ノードのラベル・プロパティが復元されること
- エッジの from/to/label が復元されること
- 100 件程度の中規模データが全て復元されること
- 強制終了（SIGKILL）後の起動でもクラッシュしないこと

### Rust テスト（オプション）

`maharit-storage` の統合テストとして、
`PersistentStorage::save_concurrent` → `load_concurrent` のラウンドトリップを
より大きなデータセットで検証するテストを追加する。

## Docker での実行方法

```bash
docker compose run --rm maharit-server \
  /app/maharit server --data /data/test.db &
# テスト実行
python3 scripts/persistence_test.py
```

## 完了条件

- [x] `scripts/persistence_test.py` が実装されていること
- [x] シャットダウン → 再起動後にノード・エッジが復元されることを検証
- [x] 正常終了（SIGTERM）と強制終了（SIGKILL）の両方をテスト
- [ ] CI パイプライン（Task 77）に組み込まれていること → Task 77 側で対応

## 解決済み (2026-06-13)

### 検証結果

`python3 scripts/persistence_test.py` を実行し全 17 件 PASS:

- SIGTERM ラウンドトリップ: 13 項目（ノード/エッジ/プロパティ/ラベル/100 件規模）
- SIGKILL 後の再起動: 4 項目（再起動成功 / SIGTERM 保存データ復元）

### Rust 統合テスト（オプション要件も達成）

`crates/maharit-storage/src/persistence.rs` の `tests::` に既に以下が実装済み:

- `test_concurrent_roundtrip_nodes_and_properties`
- `test_concurrent_roundtrip_edges`
- `test_concurrent_roundtrip_labels`
- `test_concurrent_roundtrip_large_dataset`
- `test_concurrent_roundtrip_empty_graph`

`cargo test -p maharit-storage --lib test_concurrent_roundtrip` で 5 件 PASS。

### 残課題

CI パイプライン組み込みのみ。Task 77（CI/CD E2E パイプライン）の中で
`persistence_test.py` を実行ステップとして追加すること。

## 関連ファイル

- `scripts/persistence_test.py`
- `crates/maharit-storage/src/persistence.rs` (tests モジュール)
- `crates/maharit-server/src/main.rs` (SIGTERM ハンドラ / `--data` フラグ)
