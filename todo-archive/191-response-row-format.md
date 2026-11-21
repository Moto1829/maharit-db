# 191: 応答の行を「列名 1 回 + 配列」にする（プロトコル変更）

## 問題

応答は行ごとに `HashMap<String, serde_json::Value>` を作り、列名を毎行複製する。ストリームはチャンクをさらに `to_vec()` で複製する。
10,000 行の JSON 化（`response/json_10k_properties`）が 4.77 ms で、クエリの実行（4.40 ms）と同じくらいかかる。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- 行を配列で返す形式を足す。既存の形式を残してリクエストで選べるようにするか、置き換える（`feat!:`）かを決める
- Rust / Python クライアントと docs のプロトコル説明を合わせる
- 互換を保つ範囲でできること（チャンクの複製をやめる、直接バイト列に書く）を先にやる

## 完了の目安

- `response/json_10k_*` と `STREAM MATCH` が短くなる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-21、完了）

- 置き換え（`feat!:`）ではなく、リクエストで選べるようにした（`feat:`）。既存の形式を変えると Rust / Python クライアント、可視化、E2E が全て壊れるため
  - `query` に省略可能な `rowFormat`（`"map"` 既定 / `"array"`）。`"array"` なら `{"type": "table", "columns": [...], "rows": [[...], ...]}`
  - Rust クライアント `Client::query_table` / `TableResult`、Python クライアント `execute_table` / `TableResult`（同期・非同期）
  - docs のプロトコル説明（日英）
- 互換を保つ改善: ストリーミングで全行を先に変換してからチャンクを `to_vec()` で複製していたのを、チャンクごとに変換して送るようにした（複製なし）

エンジンのベンチ（10,000 行を JSON にする）: 3 列のプロパティ `json` 4.73 ms → `table` 2.51 ms、ノード 2.96 → 2.58 ms。
