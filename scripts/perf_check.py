#!/usr/bin/env python3
"""
MaharitDB パフォーマンス回帰チェック

baseline JSON と current JSON のスループットを比較し、
指定した閾値（デフォルト 20%）以上の劣化があれば終了コード 1 を返す。

使い方:
  # ベースライン作成
  python3 scripts/benchmark.py --nodes 1000 --output-json benchmark_reports/baseline.json

  # 現在の性能を計測して比較
  python3 scripts/benchmark.py --nodes 1000 --output-json /tmp/current.json
  python3 scripts/perf_check.py benchmark_reports/baseline.json /tmp/current.json

  # 閾値を変える（10% 劣化で FAIL）
  python3 scripts/perf_check.py baseline.json current.json --threshold 0.10

CI での使い方:
  python3 scripts/benchmark.py --nodes 1000 --output-json /tmp/bench_current.json
  python3 scripts/perf_check.py benchmark_reports/baseline.json /tmp/bench_current.json
"""

import argparse
import json
import sys

# ── ANSI カラー ──────────────────────────────────────────────────────────────
GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
RESET  = "\033[0m"


def load_json(path: str) -> dict:
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        print(f"{RED}エラー: ファイルが見つかりません: {path}{RESET}", file=sys.stderr)
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(f"{RED}エラー: JSON パースに失敗しました ({path}): {e}{RESET}", file=sys.stderr)
        sys.exit(2)


def check_regression(
    baseline: dict,
    current: dict,
    threshold: float,
) -> tuple[list, list, list]:
    """
    Returns:
        failures : FAIL した操作のリスト [(name, base_tput, curr_tput, degradation), ...]
        warnings : 警告（閾値の半分以上の劣化）のリスト [(name, ...)]
        skipped  : ベースラインにあるが現在の結果にない操作名のリスト
    """
    base_results = baseline.get("results", {})
    curr_results = current.get("results", {})

    failures: list = []
    warnings: list = []
    skipped:  list = []

    for name, base_m in base_results.items():
        base_tput = base_m.get("ops_per_sec", 0.0)
        if base_tput == 0:
            continue  # ゼロ除算を避ける

        if name not in curr_results:
            skipped.append(name)
            continue

        curr_tput = curr_results[name].get("ops_per_sec", 0.0)
        degradation = 1.0 - (curr_tput / base_tput)

        if degradation >= threshold:
            failures.append((name, base_tput, curr_tput, degradation))
        elif degradation >= threshold / 2:
            warnings.append((name, base_tput, curr_tput, degradation))

    return failures, warnings, skipped


def fmt_tput(v: float) -> str:
    return f"{v:,.0f}/s"


def main():
    parser = argparse.ArgumentParser(
        description="MaharitDB パフォーマンス回帰チェック",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("baseline", help="ベースライン JSON ファイルパス")
    parser.add_argument("current",  help="現在の結果 JSON ファイルパス")
    parser.add_argument(
        "--threshold", type=float, default=0.30,
        help="FAIL とみなす劣化率（デフォルト: 0.30 = 30%%）",
    )
    args = parser.parse_args()

    baseline = load_json(args.baseline)
    current  = load_json(args.current)

    print(f"\n{BOLD}MaharitDB パフォーマンス回帰チェック{RESET}")
    print(f"  ベースライン : {args.baseline}")
    print(f"    timestamp  : {baseline.get('timestamp', 'N/A')}")
    print(f"    node_count : {baseline.get('node_count', 'N/A'):,}")
    base_env = baseline.get("environment", {})
    if base_env:
        print(f"    build_type : {base_env.get('build_type', 'N/A')}")
        if "docker_image_id" in base_env:
            print(f"    image_id   : {base_env['docker_image_id'][:19]}…")
    print(f"  現在の結果   : {args.current}")
    print(f"    timestamp  : {current.get('timestamp', 'N/A')}")
    print(f"    node_count : {current.get('node_count', 'N/A'):,}")
    curr_env = current.get("environment", {})
    if curr_env:
        print(f"    build_type : {curr_env.get('build_type', 'N/A')}")
        if "docker_image_id" in curr_env:
            print(f"    image_id   : {curr_env['docker_image_id'][:19]}…")
    print(f"  失敗閾値     : {args.threshold:.0%} 以上の劣化（WARN: {args.threshold/2:.0%} 以上）")

    # 環境不一致の警告
    base_image = base_env.get("docker_image_id")
    curr_image = curr_env.get("docker_image_id")
    if base_image and curr_image and base_image != curr_image:
        print(f"\n{YELLOW}{BOLD}⚠ 警告: ベースラインと現在の結果で Docker イメージが異なります。{RESET}")
        print(f"{YELLOW}  ベースライン image_id : {base_image[:40]}{RESET}")
        print(f"{YELLOW}  現在         image_id : {curr_image[:40]}{RESET}")
        print(f"{YELLOW}  環境差異により比較結果が不正確になる可能性があります。{RESET}")
        print(f"{YELLOW}  ベースラインを再取得することを推奨します:{RESET}")
        print(f"{YELLOW}    docker compose build maharit-server{RESET}")
        print(f"{YELLOW}    docker compose up -d maharit-server{RESET}")
        print(f"{YELLOW}    python3 scripts/benchmark.py --nodes 1000 --output-json benchmark_reports/baseline.json{RESET}")
    elif (base_env.get("build_type") or curr_env.get("build_type")) and \
            base_env.get("build_type") != curr_env.get("build_type"):
        print(f"\n{YELLOW}{BOLD}⚠ 警告: ビルドタイプが異なります "
              f"(ベースライン: {base_env.get('build_type', 'N/A')} / "
              f"現在: {curr_env.get('build_type', 'N/A')})。{RESET}")
        print(f"{YELLOW}  比較結果が不正確になる可能性があります。{RESET}")
    print()

    failures, warnings, skipped = check_regression(baseline, current, args.threshold)

    # ── 全操作の一覧表示 ──────────────────────────────────────────────────────
    print(f"{'操作':<45} {'ベースライン':>14} {'現在':>14} {'変化':>8}  判定")
    print("─" * 100)

    base_results = baseline.get("results", {})
    curr_results = current.get("results", {})

    for name, base_m in base_results.items():
        base_tput = base_m.get("ops_per_sec", 0.0)
        curr_m    = curr_results.get(name)

        if curr_m is None:
            print(f"  {name:<43} {fmt_tput(base_tput):>14}  {'N/A':>14}  {'---':>8}  {YELLOW}SKIP{RESET}")
            continue

        curr_tput   = curr_m.get("ops_per_sec", 0.0)
        degradation = 1.0 - (curr_tput / base_tput) if base_tput > 0 else 0.0
        change_str  = f"{-degradation:+.1%}"

        if degradation >= args.threshold:
            verdict = f"{RED}{BOLD}FAIL{RESET}"
        elif degradation >= args.threshold / 2:
            verdict = f"{YELLOW}WARN{RESET}"
        else:
            verdict = f"{GREEN}OK{RESET}  "

        print(
            f"  {name:<43} {fmt_tput(base_tput):>14} {fmt_tput(curr_tput):>14}"
            f"  {change_str:>8}  {verdict}"
        )

    print("─" * 100)

    # ── サマリ ───────────────────────────────────────────────────────────────
    total   = len(base_results)
    n_fail  = len(failures)
    n_warn  = len(warnings)
    n_skip  = len(skipped)
    n_ok    = total - n_fail - n_warn - n_skip

    print()
    print(f"  結果: {GREEN}OK {n_ok}{RESET}  {YELLOW}WARN {n_warn}{RESET}  {RED}FAIL {n_fail}{RESET}  SKIP {n_skip}  / 計 {total} 操作")

    if failures:
        print(f"\n{RED}{BOLD}回帰検出 — 以下の操作が {args.threshold:.0%} 以上劣化しました:{RESET}")
        for name, base_tput, curr_tput, deg in failures:
            print(f"  {RED}✗{RESET} {name}")
            print(f"      ベースライン: {fmt_tput(base_tput)}  →  現在: {fmt_tput(curr_tput)}"
                  f"  ({deg:.1%} 劣化)")
        print()
        sys.exit(1)
    else:
        print(f"\n{GREEN}{BOLD}回帰なし — すべての操作が許容範囲内です。{RESET}\n")
        sys.exit(0)


if __name__ == "__main__":
    main()
