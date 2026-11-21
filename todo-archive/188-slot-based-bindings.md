# 188: 行の束縛を HashMap から番号付きの配列にする

## 問題

`Bindings = HashMap<Arc<str>, BindingValue>` を 1 ホップ・1 行ごとに複製し、変数名の `Arc<str>` も束縛のたびに新しく確保している。
`read/hop/all_1` は 1 行あたり約 500 ns、`read/hop/all_2`（250,000 行）は 146 ms、`read/scan/return_property` は 1 行 440 ns。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- 文ごとに変数へ番号を割り当て、行を `Vec<BindingValue>`（または小さい固定長）にする
- まず変数名の `Arc<str>` を文ごとに 1 回だけ作る、という小さい段階から始めて効果を測る
- 影響範囲が広い（エグゼキュータ全体）ので、段階ごとにベンチと全テストで確かめる

## 完了の目安

- `read/hop/all_1` / `all_2` / `scan/return_property` の 1 行あたりの時間が半分以下になる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-21、完了）

段階ごとに測って進めた（4 コミット）。

1. `Bindings` を `HashMap` から「変数名と値の組の配列」（線形探索）にした。1 行の変数は数個なので、ハッシュ表の確保とハッシュ計算が無くなる
2. パターン照合で、束縛する変数名の `Arc<str>` をパターンごとに 1 回だけ作る。行の複製で、直後に足す変数の分の容量を先に取る
3. `ConcurrentGraph` の ID を鍵にする表を SipHash から乗算 1 回のハッシュ（`maharit_core::id_hash`）にした
4. `n.prop` の評価でノード全体（ラベルの一覧など）を複製せず、そのプロパティだけを読む

| 項目 | task182 時点 | 完了時 |
|---|---|---|
| read/scan/return_property | 4.40 ms（440 ns/行） | 2.23 ms（223 ns/行） |
| read/hop/all_1 | 24.8 ms | 8.2 ms |
| read/hop/all_2 | 146 ms | 52 ms |

他の項目も軒並み 10〜50% 短くなり、遅くなった項目は無い（書き込みも 5〜19% 速い）。

変数に番号を振って `Vec<BindingValue>` にする案は、エグゼキュータ全体の書き換えになる割に、上の段階でほぼ目標に届いたので行わない。
残りの時間は、プロパティ表（`HashMap<String, PropertyValue>`、SipHash）の検索と DashMap の読み取りロックが主。プロパティ表のハッシュを替えるのは `Node.properties` の公開型が変わるので、必要になったら別タスクにする。
