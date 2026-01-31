# Write-Ahead Logging (WAL)

## 概要
クラッシュリカバリのためのWAL（先行書き込みログ）を実装する。

## 実装内容

### ログレコード
- [x] レコード種別: CreateNode, DeleteNode, CreateEdge, DeleteEdge, SetProperty
- [x] レコードフォーマット: LSN, 種別, ペイロード, チェックサム
- [x] タイムスタンプ

### ログ書き込み
- [x] 追記書き込み
- [x] fsyncによる永続化保証
- [x] バッファリング戦略

### リカバリ
- [x] ログからのリプレイ
- [x] チェックポイント作成
- [ ] 古いログの削除

### API
```rust
let mut wal = Wal::open("graph.wal")?;
wal.append(RecordType::CreateNode, payload)?;
wal.sync()?;
wal.recover(&mut graph)?;
```

## 依存
- `09-persistence-format.md` が完了していること

## 対象クレート
`maharit-storage`
