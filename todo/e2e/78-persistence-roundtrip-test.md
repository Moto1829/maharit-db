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

- [ ] `scripts/persistence_test.py` が実装されていること
- [ ] シャットダウン → 再起動後にノード・エッジが復元されることを検証
- [ ] 正常終了（SIGTERM）と強制終了（SIGKILL）の両方をテスト
- [ ] CI パイプライン（Task 77）に組み込まれていること
