# バッチ書き込み対応（UNWIND + CREATE）

## 概要
現在の1リクエスト1コミット方式がCREATE のスループット律速になっている。
UNWIND を使った複数ノード/エッジの一括書き込みに対応し、書き込みスループットを10〜20倍に改善する。

## 背景（ベンチマーク根拠）
- CREATE nodes: 142/s（7 ms/op）
- 内訳: TCP往復 + WAL書き込み + ロック取得が毎回発生
- UNWIND は `execute_unwind` として既に executor に実装済みだが、複数ノード生成パターンが未対応の可能性がある

## 実装内容

### UNWIND + CREATE の対応確認・拡張
- [ ] `UNWIND $nodes AS n CREATE (:Label {id: n.id, name: n.name})` 形式が動作するか検証
- [ ] パラメータ `$nodes` に配列を渡したときの executor 処理を確認・修正
- [ ] 1クエリで 1,000件以上のノードを一括作成できることを確認

### TCP パイプライン対応（任意）
- [ ] 複数クエリを1接続で連続送信したとき、サーバーが順次処理できるか検証
- [ ] クライアント側でパイプライン送信するユーティリティを検討

### ベンチマーク追加
- [ ] `scripts/benchmark.py` に UNWIND バッチ書き込みの計測項目を追加

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — `execute_unwind`
- `crates/maharit-server/src/` — クエリハンドラ
- `scripts/benchmark.py`

## ステータス
未着手
