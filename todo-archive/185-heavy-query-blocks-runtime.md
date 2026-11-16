# 185: 重いクエリが他の接続の応答を止める

## 問題

クエリは tokio のワーカースレッド上で同期実行される（`spawn_blocking` は checkpoint と fsync だけ）。
重いクエリがコア数ぶん走ると、軽いクエリが p50 41.8 ms / p99 1,781 ms（負荷なしは 0.30 / 0.44 ms）。重いクエリが終わるまで、読み書き・ping・新しい接続の受け付けが待たされる。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- クエリの実行を blocking プール（または専用のスレッドプール）に移す
- 同時に実行する重いクエリの数に上限を付け、軽いクエリに空きを残すかを決める
- トランザクション・ストリーム・レプリケーションの経路も同じ扱いにする

## 完了の目安

- `LAT light (heavy readers = cores)` の p99 が、重いクエリ 1 本の実行時間より十分短くなる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-16）

文の実行（読み取り・書き込み・トランザクション内・ストリーム）を `spawn_blocking` で
blocking プールに移した（`run_statement`）。書き込みは `write_gate` を持ったまま待つので、順序は変わらない。

サーバーベンチ（1,000 ノード、10 コア、変更の前後を同じ条件で計測）:

| 項目 | 前 | 後 |
|---|---:|---:|
| LAT light (heavy readers = cores) p50 | 71.8 ms | 0.67 ms |
| LAT light (heavy readers = cores) p99 | 1,469 ms | 13.1 ms |
| LAT light (few heavy readers) p99 | 926 ms | 0.55 ms |
| LAT light (idle) p50 | 0.293 ms | 0.313 ms |
| PARALLEL point-lookup x1 | 3,344 /s | 3,061 /s |
| PARALLEL point-lookup x8 | 11,802 /s | 10,338 /s |

代わりに、スレッドの受け渡しで 1 クエリあたり約 10〜20 µs 増える（軽いクエリのスループットが 6〜12% 下がる）。

## 残り

- 同時に実行する重いクエリの数に上限は付けていない（blocking プールの既定の上限は 512 スレッド）
- `ROLLBACK`（undo の適用）と、結果を JSON にする部分はまだワーカースレッド上で動く
- 軽いクエリの受け渡しのコストを減らす（短い読み取りだけその場で実行する、など）
