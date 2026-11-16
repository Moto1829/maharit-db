#!/usr/bin/env bash
# MaharitDB のベンチマーク一式を Docker なしで実行する。
#
# 使い方:
#   bash scripts/run_bench_local.sh                  # エンジン単体 + サーバー
#   bash scripts/run_bench_local.sh --engine-only    # エンジン単体（cargo bench）だけ
#   bash scripts/run_bench_local.sh --server-only    # サーバー（TCP 経由）だけ
#   bash scripts/run_bench_local.sh --nodes 10000 --name before
#
# 実行内容:
#   1. エンジン単体: cargo bench -p maharit-query --bench engine
#      パーサーとエグゼキュータだけの時間（TCP・JSON・WAL を含まない）
#   2. サーバー: リリースビルドを WAL 付き（--data）で起動し scripts/benchmark.py を実行
#      TCP 経由の時間、複数接続でのスループット、負荷がある間の応答時間
#
# 結果は benchmark_reports/ に <種類>_<名前>.md / .json で保存する
# （名前の既定は日時）。2 回の結果は次のように比べる:
#   python3 scripts/perf_check.py benchmark_reports/engine_before.json benchmark_reports/engine_after.json
#   python3 scripts/perf_check.py benchmark_reports/server_before.json benchmark_reports/server_after.json

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

NODES=1000
PORT=17687
NAME="$(date +%Y%m%d_%H%M%S)"
RUN_ENGINE=1
RUN_SERVER=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --nodes) NODES="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        --name) NAME="$2"; shift 2 ;;
        --engine-only) RUN_SERVER=0; shift ;;
        --server-only) RUN_ENGINE=0; shift ;;
        *) echo "不明なオプション: $1"; exit 2 ;;
    esac
done

OUT=benchmark_reports
mkdir -p "$OUT"

if [[ "$RUN_ENGINE" == 1 ]]; then
    echo "▶ エンジン単体ベンチ"
    since="$(date +%s)"
    cargo bench -p maharit-query --bench engine
    python3 scripts/engine_bench_report.py --since "$since" \
        --output "$OUT/engine_$NAME.md" --output-json "$OUT/engine_$NAME.json"
fi

if [[ "$RUN_SERVER" == 1 ]]; then
    echo "▶ サーバーベンチ（ノード数 ${NODES}、ポート ${PORT}）"
    cargo build --release -p maharit-server
    TARGET_DIR="${CARGO_TARGET_DIR:-target}"
    DATA_DIR="$(mktemp -d)"
    SERVER_PID=""
    cleanup() {
        # 終了時の保存が済むのを待ってから消す
        if [[ -n "$SERVER_PID" ]]; then
            kill "$SERVER_PID" 2>/dev/null || true
            wait "$SERVER_PID" 2>/dev/null || true
        fi
        rm -rf "$DATA_DIR"
    }
    trap cleanup EXIT

    "$TARGET_DIR/release/maharit" server --host 127.0.0.1 --port "$PORT" \
        --data "$DATA_DIR/bench.db" > "$DATA_DIR/server.log" 2>&1 &
    SERVER_PID=$!
    for _ in $(seq 1 60); do
        python3 -c "import socket; socket.create_connection(('127.0.0.1', $PORT), 1)" 2>/dev/null && break
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then
            echo "エラー: サーバーが起動しません"; cat "$DATA_DIR/server.log"; exit 1
        fi
        sleep 0.5
    done

    MAHARIT_E2E_LOCAL=1 python3 scripts/benchmark.py --host 127.0.0.1 --port "$PORT" \
        --nodes "$NODES" --output "$OUT/server_$NAME.md" --output-json "$OUT/server_$NAME.json"
fi
