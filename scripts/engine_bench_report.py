#!/usr/bin/env python3
"""
エンジン単体ベンチ（cargo bench -p maharit-query --bench engine）の結果をまとめる。

criterion が target/criterion/ に書いた計測値（中央値）を読み、Markdown の表と、
perf_check.py で比べられる JSON にする。

使い方:
  cargo bench -p maharit-query --bench engine
  python3 scripts/engine_bench_report.py                       # 表を表示
  python3 scripts/engine_bench_report.py --output-json benchmark_reports/engine_before.json
  python3 scripts/engine_bench_report.py --compare benchmark_reports/engine_before.json

  # 20% 以上遅くなった項目があれば失敗させる
  python3 scripts/perf_check.py benchmark_reports/engine_before.json benchmark_reports/engine_after.json
"""

import argparse
import json
import os
import sys
from datetime import datetime, timezone


def load_results(criterion_dir: str, since: float) -> dict[str, float]:
    """ベンチ名 → 中央値（ナノ秒）。since より前に書かれた結果は古いものとして除く。"""
    results: dict[str, float] = {}
    for root, _dirs, files in os.walk(criterion_dir):
        if os.path.basename(root) != "new" or "estimates.json" not in files:
            continue
        estimates_path = os.path.join(root, "estimates.json")
        if os.path.getmtime(estimates_path) < since:
            continue
        with open(os.path.join(root, "benchmark.json"), encoding="utf-8") as f:
            name = json.load(f)["full_id"]
        with open(estimates_path, encoding="utf-8") as f:
            results[name] = json.load(f)["median"]["point_estimate"]
    return dict(sorted(results.items()))


def human(ns: float) -> str:
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.2f} {unit}"
    return f"{ns:.0f} ns"


def main() -> int:
    parser = argparse.ArgumentParser(description="エンジン単体ベンチの結果をまとめる")
    parser.add_argument("--criterion-dir", default=None,
                        help="criterion の出力先（省略時は $CARGO_TARGET_DIR/criterion または target/criterion）")
    parser.add_argument("--since", type=float, default=0.0,
                        help="この UNIX 時刻より後に計測された項目だけを載せる")
    parser.add_argument("--compare", default=None, metavar="JSON",
                        help="以前の --output-json と比べ、何倍速くなったかを表に加える")
    parser.add_argument("--output", default=None, help="Markdown の出力先")
    parser.add_argument("--output-json", default=None, help="JSON の出力先")
    args = parser.parse_args()

    criterion_dir = args.criterion_dir or os.path.join(
        os.environ.get("CARGO_TARGET_DIR", "target"), "criterion")
    results = load_results(criterion_dir, args.since)
    if not results:
        print(f"エラー: {criterion_dir} に計測結果がありません。先に cargo bench を実行してください。",
              file=sys.stderr)
        return 2

    previous: dict = {}
    if args.compare:
        with open(args.compare, encoding="utf-8") as f:
            previous = json.load(f).get("results", {})

    now = datetime.now(timezone.utc)
    lines = [
        "# MaharitDB エンジン単体ベンチマーク",
        "",
        f"- **実行日時**: {now.strftime('%Y-%m-%d %H:%M:%S UTC')}",
        "- **計測**: `cargo bench -p maharit-query --bench engine`（1 回あたりの時間の中央値）",
    ]
    if args.compare:
        lines.append(f"- **比較対象**: `{args.compare}`（倍率は 1 より大きいほど速くなった）")
    lines.append("")
    if args.compare:
        lines += ["| 項目 | 時間 | 以前 | 倍率 |", "|------|-----:|-----:|-----:|"]
    else:
        lines += ["| 項目 | 時間 | 回/秒 |", "|------|-----:|------:|"]
    for name, ns in results.items():
        if args.compare:
            before = previous.get(name, {}).get("median_ns")
            if before:
                lines.append(f"| {name} | {human(ns)} | {human(before)} | {before / ns:.2f}x |")
            else:
                lines.append(f"| {name} | {human(ns)} | - | - |")
        else:
            lines.append(f"| {name} | {human(ns)} | {1e9 / ns:,.0f} |")
    report = "\n".join(lines) + "\n"
    print(report)

    for path, text in (
        (args.output, report),
        (args.output_json, json.dumps({
            "timestamp": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "environment": {"build_type": "cargo_bench"},
            "results": {
                name: {"median_ns": round(ns, 1), "ops_per_sec": round(1e9 / ns, 2)}
                for name, ns in results.items()
            },
        }, ensure_ascii=False, indent=2) + "\n"),
    ):
        if path:
            os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            print(f"保存しました: {path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
