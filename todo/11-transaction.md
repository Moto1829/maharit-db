# トランザクション

## 概要
ACID特性を持つトランザクション機能を実装する。

## 実装内容

### 基本トランザクション
- [ ] BEGIN / COMMIT / ROLLBACK
- [ ] トランザクションID管理
- [ ] 分離レベル（Read Committed から開始）

### 同時実行制御
- [ ] 悲観的ロック（読み取り/書き込みロック）
- [ ] ロック粒度（ノード単位、グラフ単位）
- [ ] デッドロック検出

### MVCC（将来的）
- [ ] バージョンチェーン
- [ ] スナップショット分離
- [ ] ガベージコレクション

### API
```rust
let tx = db.begin_transaction();
tx.create_node("Person")?;
tx.commit()?;
// or tx.rollback()?;
```

## 依存
- `10-wal.md` が完了していること（Atomicity/Durability）

## 対象クレート
新規 `maharit-tx` または `maharit-storage`
