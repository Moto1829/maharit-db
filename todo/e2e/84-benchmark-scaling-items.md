# e2e/84: 大規模ノードのベンチ項目追加（ラベル索引・WHEREプッシュダウンの可視化）

## 概要
既存ベンチは全ノードが単一ラベル `BenchPerson` で、複数ラベル環境での
ラベル索引（query-core/81）や WHERE プッシュダウン（query-core/83）の効果が
見えなかった。大規模・多ラベルのスケーリング項目を追加する。

## 対応
- `scripts/benchmark.py` に `--scaling` フラグと `bench_scaling_label_and_pushdown()` を追加。
  - 大量の `ScaleFiller`（`grp` プロパティ、選択度 1/100）と少数の `ScaleRare` を作成。
  - 計測項目:
    - `SCALE label scan (Rare ...)`: `MATCH (n:ScaleRare)` → ラベル索引で O(該当)。
    - `SCALE WHERE grp=7 (pushdown)`: `MATCH (n:ScaleFiller) WHERE n.grp=7` → 述語プッシュダウン。
    - `SCALE inline {grp:7} (baseline)`: 同義インラインプロパティ（比較用ベースライン）。
  - セクション自身でデータをクリーンアップ。finally でも取りこぼしを掃除。
- 使い方例・ヘルプを更新（`--nodes 100000 --scaling`）。

## 効果の見方（before/after）
現在稼働中の docker コンテナは最適化前のバイナリのため、そのまま実行しても差は出ない。
before/after を可視化する手順:
```bash
# after: 新コードでイメージを再ビルドしてから
docker compose up -d --build maharit-server
python3 scripts/benchmark.py --nodes 100000 --scaling \
  --output benchmark_reports/bench_after.md
```
選択的ラベル/プッシュダウン項目のスループットが、ノード総数を増やしても
維持される（＝ O(全ノード) にならない）ことを確認する。

## ステータス
完了（py_compile OK、ヘルプに --scaling 反映。実測は稼働環境での再ビルド後に実施）
