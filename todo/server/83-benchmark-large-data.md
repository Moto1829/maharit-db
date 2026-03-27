# Task 83: 大量データ性能確認スクリプト

## 概要
`scripts/benchmark.py` を作成し、大量データに対するMaharitDBの性能（スループット・レイテンシ）を計測する。

## 実施内容
- [x] `scripts/benchmark.py` を作成
  - CREATE スループット計測（ノード・エッジ）
  - MATCH スキャン性能計測
  - WHERE フィルタ性能計測
  - リレーションシップトラバーサル性能計測
  - 集計クエリ性能計測
  - データ件数をCLIで指定可能（`--nodes`）

## 実行方法
```bash
# サーバー起動後
python3 scripts/benchmark.py
python3 scripts/benchmark.py --nodes 10000 --host localhost --port 7687
```

## ステータス
完了
