#!/usr/bin/env python3
"""
MaharitDB 大量データ性能ベンチマーク
起動中の maharit-db サーバーに対して大量データを投入し、各種クエリの性能を計測する。

使い方:
  python3 scripts/benchmark.py
  python3 scripts/benchmark.py --nodes 10000 --host localhost --port 7687
  python3 scripts/benchmark.py --nodes 1000 --no-cleanup
  python3 scripts/benchmark.py --output reports/bench_20260320.md
  python3 scripts/benchmark.py --nodes 100000 --scaling   # 大規模: ラベル索引/プッシュダウン
  python3 scripts/benchmark.py --parallel 16              # 並行項目の接続数を変える
  python3 scripts/benchmark.py --no-parallel              # 並行項目を省く

サーバーの起動からまとめて実行するには bash scripts/run_bench_local.sh を使う。

分析:
  ベンチ完了後、Claude Code で /analyze-benchmark を実行すると
  最新レポートを自動で読み込んで考察を追記します。
"""

import argparse
import json
import multiprocessing
import os
import random
import socket
import struct
import subprocess
import sys
import time
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import BOLD, CYAN, GREEN, RED, RESET, YELLOW  # noqa: E402

NAMES   = ["Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi",
           "Ivan", "Judy", "Karl", "Laura", "Mallory", "Niaj", "Olivia", "Peggy"]
CITIES  = ["Tokyo", "Osaka", "Nagoya", "Sapporo", "Fukuoka", "Kyoto",
           "Yokohama", "Kobe", "Sendai", "Hiroshima"]
SKILLS  = ["Rust", "Python", "Go", "Java", "TypeScript", "C++", "Haskell",
           "Scala", "Elixir", "Ruby"]


# ── 繰り返し計測定数 ──────────────────────────────────────────────────────────
REPEAT_MIN_SECS = 1.0   # 高速クエリを少なくともこの秒数分繰り返す
WARMUP_ITERS    = 3     # ウォームアップ回数（計測対象外）


def _run_timed(client: "MaharitClient", cypher: str, min_secs: float = REPEAT_MIN_SECS) -> tuple[int, float]:
    """単一クエリを繰り返し実行し (件数, 中央値レイテンシ) を返す。
    合計時間が min_secs を超えるまで実行し、外れ値を排除するために中央値を使用する。"""
    resp = None
    for _ in range(WARMUP_ITERS):
        resp = client.query(cypher)
    times: list[float] = []
    deadline = time.perf_counter() + min_secs
    while time.perf_counter() < deadline or len(times) < 5:
        t0 = time.perf_counter()
        resp = client.query(cypher)
        times.append(time.perf_counter() - t0)
    count = len(resp.get("rows", [])) if resp else 0
    return count, sorted(times)[len(times) // 2]


def _run_timed_stream(client: "MaharitClient", request: dict, min_secs: float = REPEAT_MIN_SECS) -> tuple[int, float]:
    """streamQuery を繰り返し実行し (総行数, 中央値レイテンシ) を返す。"""
    messages = None
    for _ in range(WARMUP_ITERS):
        client.send(request)
    times: list[float] = []
    deadline = time.perf_counter() + min_secs
    while time.perf_counter() < deadline or len(times) < 5:
        t0 = time.perf_counter()
        messages = client.send(request)
        times.append(time.perf_counter() - t0)
    chunks = [m for m in messages if m.get("type") == "streamChunk"] if messages else []
    total_rows = sum(len(c.get("rows", [])) for c in chunks)
    return total_rows, sorted(times)[len(times) // 2]


# ── 計測ユーティリティ ─────────────────────────────────────────────────────────

class BenchResult:
    def __init__(self, name: str, count: int, elapsed: float):
        self.name    = name
        self.count   = count
        self.elapsed = elapsed

    @property
    def throughput(self) -> float:
        return self.count / self.elapsed if self.elapsed > 0 else 0.0

    @property
    def latency_ms(self) -> float:
        return (self.elapsed / self.count * 1000) if self.count > 0 else 0.0

    def print(self):
        print(
            f"  {GREEN}✓{RESET} {self.name:<40} "
            f"{YELLOW}{self.count:>7,}{RESET} ops  "
            f"{CYAN}{self.elapsed:>6.2f}s{RESET}  "
            f"{GREEN}{self.throughput:>8,.0f} ops/s{RESET}  "
            f"avg {self.latency_ms:.2f} ms/op"
        )


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


def progress(current: int, total: int, width: int = 30):
    pct  = current / total
    done = int(pct * width)
    bar  = "█" * done + "░" * (width - done)
    print(f"\r  [{bar}] {current:,}/{total:,}", end="", flush=True)


# ── ベンチマーク本体 ──────────────────────────────────────────────────────────

def bench_create_nodes(client: MaharitClient, n: int) -> BenchResult:
    """Person ノードを n 件 CREATE する"""
    section(f"CREATE ノード x {n:,}")
    start = time.perf_counter()
    for i in range(n):
        name  = NAMES[i % len(NAMES)] + str(i)
        age   = 20 + (i % 50)
        city  = CITIES[i % len(CITIES)]
        skill = SKILLS[i % len(SKILLS)]
        client.query(
            f"CREATE (:BenchPerson {{id: {i}, name: '{name}', age: {age}, "
            f"city: '{city}', skill: '{skill}'}})"
        )
        if (i + 1) % max(1, n // 20) == 0:
            progress(i + 1, n)
    print()
    elapsed = time.perf_counter() - start
    r = BenchResult("CREATE Person nodes", n, elapsed)
    r.print()
    return r


def bench_create_edges(client: MaharitClient, n: int, edge_ratio: float = 0.3) -> BenchResult:
    """ランダムな KNOWS エッジを作成する（ノード数の edge_ratio 倍）"""
    num_edges = max(1, int(n * edge_ratio))
    section(f"CREATE エッジ x {num_edges:,} (ノード数の {edge_ratio:.0%})")

    # ノード id 一覧を取得
    resp = client.query("MATCH (p:BenchPerson) RETURN p.id")
    ids  = [int(row["p.id"]) for row in resp.get("rows", []) if row.get("p.id") is not None]
    if len(ids) < 2:
        print(f"  {RED}✗ ノード不足でスキップ{RESET}")
        return BenchResult("CREATE KNOWS edges", 0, 0.001)

    start = time.perf_counter()
    created = 0
    for i in range(num_edges):
        a, b = random.sample(ids[:min(len(ids), 200)], 2)  # 最初の200件を対象
        resp = client.query(
            f"MATCH (a:BenchPerson {{id: {a}}}), (b:BenchPerson {{id: {b}}}) "
            f"CREATE (a)-[:BENCH_KNOWS {{since: {2000 + (i % 25)}}}]->(b)"
        )
        if resp.get("type") == "result":
            created += 1
        if (i + 1) % max(1, num_edges // 20) == 0:
            progress(i + 1, num_edges)
    print()
    elapsed = time.perf_counter() - start
    r = BenchResult("CREATE KNOWS edges", created, elapsed)
    r.print()
    return r


def bench_full_scan(client: MaharitClient) -> BenchResult:
    """全 BenchPerson ノードをスキャンする"""
    section("MATCH フルスキャン")
    count, med = _run_timed(client, "MATCH (n:BenchPerson) RETURN n")
    r = BenchResult("MATCH full scan (BenchPerson)", count, med)
    r.print()
    return r


def bench_filter_queries(client: MaharitClient, n: int) -> list[BenchResult]:
    """WHERE フィルタ付きクエリを複数パターン計測する"""
    section("WHERE フィルタクエリ")
    results = []

    patterns = [
        ("WHERE age > 40",       "MATCH (n:BenchPerson) WHERE n.age > 40 RETURN n"),
        ("WHERE city = 'Tokyo'", "MATCH (n:BenchPerson) WHERE n.city = 'Tokyo' RETURN n"),
        ("WHERE skill = 'Rust'", "MATCH (n:BenchPerson) WHERE n.skill = 'Rust' RETURN n"),
        ("WHERE id < 100",       "MATCH (n:BenchPerson) WHERE n.id < 100 RETURN n"),
    ]

    for label, cypher in patterns:
        count, med = _run_timed(client, cypher)
        r = BenchResult(f"MATCH {label}", count, med)
        r.print()
        results.append(r)

    return results


def bench_aggregation(client: MaharitClient) -> list[BenchResult]:
    """COUNT / AVG などの集計クエリを計測する"""
    section("集計クエリ")
    results = []

    agg_queries = [
        ("COUNT all",       "MATCH (n:BenchPerson) RETURN count(n)"),
        ("AVG age",         "MATCH (n:BenchPerson) RETURN avg(n.age)"),
        ("COUNT per city",  "MATCH (n:BenchPerson) RETURN n.city, count(n)"),
        ("COUNT per skill", "MATCH (n:BenchPerson) RETURN n.skill, count(n)"),
    ]

    for label, cypher in agg_queries:
        count, med = _run_timed(client, cypher)
        r = BenchResult(f"AGG {label}", count, med)
        r.print()
        results.append(r)

    return results


def bench_traversal(client: MaharitClient) -> list[BenchResult]:
    """リレーションシップトラバーサルを計測する"""
    section("リレーションシップトラバーサル")
    results = []

    trav_queries = [
        ("1-hop KNOWS",    "MATCH (a:BenchPerson)-[:BENCH_KNOWS]->(b:BenchPerson) RETURN a, b"),
        ("filter on edge", "MATCH (a:BenchPerson)-[r:BENCH_KNOWS]->(b:BenchPerson) WHERE r.since > 2010 RETURN a, b"),
        ("2-hop KNOWS",    "MATCH (a:BenchPerson)-[:BENCH_KNOWS]->(b:BenchPerson)-[:BENCH_KNOWS]->(c:BenchPerson) RETURN a.id, c.id"),
    ]

    for label, cypher in trav_queries:
        count, med = _run_timed(client, cypher)
        r = BenchResult(f"TRAV {label}", count, med)
        r.print()
        results.append(r)

    return results


def bench_stream(client: MaharitClient, chunk_size: int = 100) -> BenchResult:
    """ストリーミングで全ノードを取得する"""
    section(f"ストリーミング (chunkSize={chunk_size})")
    request = {
        "type":      "streamQuery",
        "query":     "MATCH (n:BenchPerson) RETURN n",
        "chunkSize": chunk_size,
    }
    total_rows, med = _run_timed_stream(client, request)
    r = BenchResult(f"STREAM MATCH (chunk={chunk_size})", total_rows, med)
    r.print()
    return r


def bench_concurrent_reads(client: MaharitClient, repeat: int = 50) -> BenchResult:
    """同一クライアントで繰り返し読み取りクエリを送信する"""
    section(f"連続読み取り x {repeat}")
    start = time.perf_counter()
    for i in range(repeat):
        client.query(f"MATCH (n:BenchPerson) WHERE n.id = {i % 200} RETURN n")
    elapsed = time.perf_counter() - start
    r = BenchResult("Repeated point-lookup (id filter)", repeat, elapsed)
    r.print()
    return r


# ── 並行ベンチマーク（複数接続）───────────────────────────────────────────────
#
# 負荷をかける側は接続ごとに別プロセスにする（スレッドだと Python の GIL で
# クライアント側が先に頭打ちになり、サーバーの並行性が測れない）。

PARALLEL_SECS = 2.0      # 並行項目 1 つあたりの計測時間
TMP_LABEL     = "BenchTmp"

# 軽いクエリ（1 件を返す）と、重いクエリ（全ノードの直積を数える）。
LIGHT_QUERY = "MATCH (n:BenchPerson) WHERE n.id = 7 RETURN n.name"
HEAVY_QUERY = ("MATCH (a:BenchPerson), (b:BenchPerson) "
               "WHERE a.age + b.age > 130 RETURN count(*)")
BATCH_WRITE = f"UNWIND range(1, 200) AS i CREATE (:{TMP_LABEL} {{id: i}})"


def _load_worker(host: str, port: int, queries: list[str], start_at: float,
                 stop_at: float | None, stop_event, counts) -> None:
    """queries を順に繰り返し送り続け、成功した回数を counts に入れる。

    stop_at があればその時刻まで、無ければ stop_event が立つまで続ける。"""
    client = MaharitClient(host, port, timeout=120.0)
    done = 0
    try:
        while time.time() < start_at:
            time.sleep(0.001)
        i = 0
        while True:
            if stop_at is not None:
                if time.time() >= stop_at:
                    break
            elif stop_event.is_set():
                break
            resp = client.query(queries[i % len(queries)])
            if resp.get("type") == "result":
                done += 1
            i += 1
    finally:
        counts.put(done)
        client.close()


def _parallel_throughput(host: str, port: int, conns: int,
                         queries_for: "callable", secs: float = PARALLEL_SECS) -> tuple[int, float]:
    """conns 本の接続から secs 秒間クエリを送り続け (合計回数, 秒数) を返す。"""
    ctx = multiprocessing.get_context("spawn")
    counts = ctx.Queue()
    start_at = time.time() + 1.0          # 全プロセスが接続し終えるのを待つ
    stop_at = start_at + secs
    procs = [
        ctx.Process(target=_load_worker,
                    args=(host, port, queries_for(w), start_at, stop_at, None, counts))
        for w in range(conns)
    ]
    for p in procs:
        p.start()
    total = sum(counts.get(timeout=secs + 120) for _ in procs)
    for p in procs:
        p.join()
    return total, secs


class _BackgroundLoad:
    """with ブロックの間、conns 本の接続から queries を送り続ける。"""

    def __init__(self, host: str, port: int, conns: int, queries: list[str]):
        ctx = multiprocessing.get_context("spawn")
        self.stop = ctx.Event()
        self.counts = ctx.Queue()
        start_at = time.time()
        self.procs = [
            ctx.Process(target=_load_worker,
                        args=(host, port, queries, start_at, None, self.stop, self.counts))
            for _ in range(conns)
        ]

    def __enter__(self):
        for p in self.procs:
            p.start()
        time.sleep(1.0)                   # 負荷が立ち上がるのを待つ
        return self

    def __exit__(self, exc_type, exc, tb):
        self.stop.set()
        self.completed = sum(self.counts.get(timeout=180) for _ in self.procs)
        for p in self.procs:
            p.join()


def _latencies(client: MaharitClient, cypher: str, secs: float = PARALLEL_SECS,
               min_samples: int = 20) -> list[float]:
    """cypher を secs 秒間（少なくとも min_samples 回）繰り返し、1 回ごとの所要時間（秒）を昇順で返す。"""
    times: list[float] = []
    deadline = time.perf_counter() + secs
    while time.perf_counter() < deadline or len(times) < min_samples:
        t0 = time.perf_counter()
        client.query(cypher)
        times.append(time.perf_counter() - t0)
    return sorted(times)


def _latency_results(name: str, times: list[float]) -> list[BenchResult]:
    """p50 / p99 を「1 回あたりの時間」として BenchResult にする。"""
    results = []
    for label, q in (("p50", 0.50), ("p99", 0.99)):
        value = times[min(len(times) - 1, int(len(times) * q))]
        r = BenchResult(f"LAT {name} {label}", 1, value)
        r.print()
        results.append(r)
    return results


def bench_parallel(client: MaharitClient, host: str, port: int, conns: int) -> list[BenchResult]:
    """複数接続でのスループットと、他の負荷がある間の軽いクエリの応答時間を測る。"""
    results: list[BenchResult] = []

    def point_lookups(worker: int) -> list[str]:
        return [f"MATCH (n:BenchPerson) WHERE n.id = {(worker * 37 + k) % 200} RETURN n"
                for k in range(50)]

    def creates(worker: int) -> list[str]:
        return [f"CREATE (:{TMP_LABEL} {{id: {worker}}})"]

    # 接続数を増やしたときに読み取り・書き込みのスループットがどれだけ伸びるか
    section(f"並行スループット（1 接続 / {conns} 接続）")
    for label, queries_for in (("point-lookup", point_lookups), ("CREATE", creates)):
        for n in (1, conns):
            total, secs = _parallel_throughput(host, port, n, queries_for)
            r = BenchResult(f"PARALLEL {label} x{n}", total, secs)
            r.print()
            results.append(r)
    client.query(f"MATCH (n:{TMP_LABEL}) DELETE n")

    # 重いクエリや書き込みが走っている間に、軽いクエリがどれだけ待たされるか
    section("軽いクエリの応答時間（他の負荷がある間）")
    results.extend(_latency_results("light (idle)", _latencies(client, LIGHT_QUERY)))

    # 重いクエリが CPU コア数より少ないとき（空きがある）と、コア数と同じとき
    # （サーバーのワーカースレッドがすべて重いクエリで埋まる）の両方を測る。
    cores = os.cpu_count() or 4
    for label, heavy_conns in (("few heavy readers", max(1, cores // 4)),
                               ("heavy readers = cores", cores)):
        with _BackgroundLoad(host, port, heavy_conns, [HEAVY_QUERY]) as load:
            times = _latencies(client, LIGHT_QUERY, min_samples=5)
        results.extend(_latency_results(f"light ({label})", times))
        print(f"    重いクエリ {heavy_conns} 接続、完了数: {load.completed}")

    with _BackgroundLoad(host, port, 2, [BATCH_WRITE]) as load:
        times = _latencies(client, LIGHT_QUERY)
    results.extend(_latency_results("light (2 batch writers)", times))
    print(f"    書き込みバッチの完了数: {load.completed}")
    client.query(f"MATCH (n:{TMP_LABEL}) DELETE n")

    return results


def bench_unwind_batch_create(client: MaharitClient, n: int) -> BenchResult:
    """UNWIND + CREATE で n 件を1クエリで一括作成する"""
    section(f"UNWIND バッチ書き込み x {n:,}")

    # バッチサイズごとに送信（サーバーのメッセージサイズ制限を考慮して 500件ずつ）
    batch_size = 500
    total_created = 0
    start = time.perf_counter()

    for batch_start in range(0, n, batch_size):
        batch_end = min(batch_start + batch_size, n)
        items = [
            {"id": i, "name": NAMES[i % len(NAMES)] + str(i), "city": CITIES[i % len(CITIES)]}
            for i in range(batch_start, batch_end)
        ]
        import json
        items_json = json.dumps(items)
        resp = client.query(
            f"UNWIND {items_json} AS item "
            f"CREATE (:UnwindBench {{id: item.id, name: item.name, city: item.city}})"
        )
        if resp.get("type") == "result":
            total_created += batch_end - batch_start
        if batch_end % max(1, (n // 20)) < batch_size:
            progress(batch_end, n)

    print()
    elapsed = time.perf_counter() - start
    r = BenchResult("UNWIND batch CREATE (map list)", total_created, elapsed)
    r.print()

    # クリーンアップ
    client.query("MATCH (n:UnwindBench) DELETE n")
    return r


def bench_scaling_label_and_pushdown(client: MaharitClient, n: int) -> list[BenchResult]:
    """多ラベル大規模データで、ラベル索引と WHERE プッシュダウンの効果を可視化する。

    - 大量の `ScaleFiller`（`grp` プロパティ、選択度 1/100）と少数の `ScaleRare` を作成。
    - 選択的ラベルスキャン `MATCH (n:ScaleRare)` はラベル索引により O(該当) を期待。
    - `WHERE n.grp = 7` は述語プッシュダウンで、同義のインライン `{grp:7}` と同等の性能を期待。

    大規模ノード数（例: `--nodes 100000 --scaling`）で実行すると差が顕著になる。
    """
    import json

    section(f"スケーリング: 多ラベル {n:,} 件 + 選択的ラベル")

    # 大量の Filler ノードをバッチ作成（grp: 0..99 の 1/100 選択度）
    batch_size = 500
    for batch_start in range(0, n, batch_size):
        batch_end = min(batch_start + batch_size, n)
        items = [{"id": i, "grp": i % 100} for i in range(batch_start, batch_end)]
        client.query(
            f"UNWIND {json.dumps(items)} AS it "
            f"CREATE (:ScaleFiller {{id: it.id, grp: it.grp}})"
        )
        if batch_end % max(1, (n // 20)) < batch_size:
            progress(batch_end, n)
    print()

    # 少数の Rare ノード（全体の中で選択的: 約 0.1%、最低 10 件）
    rare = max(10, n // 1000)
    rare_items = [{"id": i} for i in range(rare)]
    client.query(
        f"UNWIND {json.dumps(rare_items)} AS it CREATE (:ScaleRare {{id: it.id}})"
    )

    results: list[BenchResult] = []

    # 選択的ラベルスキャン（ラベル索引 → 全走査せず該当ノードのみ列挙）
    count, med = _run_timed(client, "MATCH (n:ScaleRare) RETURN n")
    r = BenchResult(f"SCALE label scan (Rare {rare} of {n:,})", count, med)
    r.print()
    results.append(r)

    # WHERE 等価（述語プッシュダウン、grp は 1/100 選択度）
    count, med = _run_timed(client, "MATCH (n:ScaleFiller) WHERE n.grp = 7 RETURN n")
    r = BenchResult("SCALE WHERE grp=7 (pushdown)", count, med)
    r.print()
    results.append(r)

    # 同義のインラインプロパティ（プッシュダウンの比較用ベースライン）
    count, med = _run_timed(client, "MATCH (n:ScaleFiller {grp: 7}) RETURN n")
    r = BenchResult("SCALE inline {grp:7} (baseline)", count, med)
    r.print()
    results.append(r)

    # クリーンアップ（このセクションのデータのみ）
    client.query("MATCH (n:ScaleFiller) DELETE n")
    client.query("MATCH (n:ScaleRare) DELETE n")
    return results


def cleanup(client: MaharitClient):
    section("クリーンアップ")
    resp = client.query("MATCH (n:BenchPerson) DETACH DELETE n")
    ok   = resp.get("type") == "result"
    mark = f"{GREEN}✓{RESET}" if ok else f"{RED}✗{RESET}"
    print(f"  {mark} BenchPerson + BENCH_KNOWS を削除")


# ── サマリ出力 ─────────────────────────────────────────────────────────────────

def print_summary(all_results: list[BenchResult]):
    print(f"\n{'─' * 80}")
    print(f"{BOLD}{'ベンチマーク結果サマリ':^80}{RESET}")
    print(f"{'─' * 80}")
    print(f"  {'操作':<40} {'件数':>8}  {'時間':>7}  {'スループット':>12}  {'レイテンシ':>10}")
    print(f"  {'─' * 76}")
    for r in all_results:
        if r.count == 0:
            continue
        print(
            f"  {r.name:<40} {r.count:>8,}  {r.elapsed:>6.2f}s  "
            f"{r.throughput:>10,.0f}/s  {r.latency_ms:>8.2f}ms"
        )
    print(f"{'─' * 80}")


def _build_report_lines(all_results: list[BenchResult], args, now: str) -> list[str]:
    """レポート本文（Markdown 行リスト）を構築する"""
    lines = [
        "# MaharitDB ベンチマークレポート",
        "",
        f"- **実行日時**: {now}",
        f"- **接続先**: `{args.host}:{args.port}`",
        f"- **ノード数**: {args.nodes:,}",
        f"- **エッジ比率**: {args.edge_ratio:.0%}",
        f"- **連続読み取り回数**: {args.repeat}",
        f"- **ストリームチャンクサイズ**: {args.chunk_size}",
        f"- **並行項目の接続数**: {'なし' if args.no_parallel else args.parallel}",
        "",
        "## 計測結果",
        "",
        "| 操作 | 件数 | 時間 (s) | スループット (/s) | レイテンシ (ms/op) |",
        "|------|-----:|---------:|----------------:|------------------:|",
    ]

    for r in all_results:
        if r.count == 0:
            continue
        lines.append(
            f"| {r.name} | {r.count:,} | {r.elapsed:.2f} "
            f"| {r.throughput:,.0f} | {r.latency_ms:.2f} |"
        )

    return lines


def save_report(all_results: list[BenchResult], args, output_path: str):
    """Markdown レポートをファイルに書き出す"""
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
    lines = _build_report_lines(all_results, args, now)
    lines += ["", "---", "*Generated by `scripts/benchmark.py`*", ""]

    os.makedirs(os.path.dirname(output_path) if os.path.dirname(output_path) else ".", exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))

    print(f"\n  {GREEN}✓{RESET} レポートを保存しました: {BOLD}{output_path}{RESET}")
    print(f"  {CYAN}ℹ{RESET}  分析するには Claude Code で {BOLD}/analyze-benchmark{RESET} を実行してください。")


def get_docker_image_id(container_name: str = "maharit-db-server") -> str | None:
    """起動中コンテナのイメージ ID を取得する（取得できない場合は None）"""
    try:
        result = subprocess.run(
            ["docker", "inspect", "--format", "{{.Image}}", container_name],
            capture_output=True, text=True, timeout=5,
        )
        image_id = result.stdout.strip()
        return image_id if image_id else None
    except Exception:
        return None


def save_json_report(all_results: list[BenchResult], args, output_path: str):
    """JSON フォーマットでベンチマーク結果を保存する（perf_check.py で利用）"""
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    results = {}
    for r in all_results:
        if r.count == 0:
            continue
        results[r.name] = {
            "ops_per_sec": round(r.throughput, 2),
            "latency_ms":  round(r.latency_ms, 4),
            "count":       r.count,
            "elapsed_s":   round(r.elapsed, 4),
        }

    if os.environ.get("MAHARIT_E2E_LOCAL"):
        environment: dict = {"build_type": "local"}
    else:
        environment = {"build_type": "docker_release"}
        image_id = get_docker_image_id()
        if image_id:
            environment["docker_image_id"] = image_id

    payload = {
        "timestamp":   now,
        "node_count":  args.nodes,
        "host":        args.host,
        "port":        args.port,
        "environment": environment,
        "results":     results,
    }

    os.makedirs(os.path.dirname(output_path) if os.path.dirname(output_path) else ".", exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
        f.write("\n")

    print(f"  {GREEN}✓{RESET} JSON レポートを保存しました: {BOLD}{output_path}{RESET}")


# ── エントリポイント ──────────────────────────────────────────────────────────

def check_docker_compose() -> None:
    """docker-compose.yml の maharit-db-server が起動中か確認する。"""
    # CI やローカルプロセスで起動したサーバーに対して実行する場合は確認しない
    if os.environ.get("MAHARIT_E2E_LOCAL"):
        return
    try:
        result = subprocess.run(
            ["docker", "ps", "--format", "{{.Names}}"],
            capture_output=True, text=True, timeout=5,
        )
        running = set(result.stdout.splitlines())
    except Exception:
        print(f"{YELLOW}警告: Docker が利用できないため前提環境の確認をスキップします。{RESET}")
        return

    if "maharit-db-server" in running:
        return

    if running & {"maharit-leader", "maharit-follower1", "maharit-follower2"}:
        print(f"\n{RED}エラー: docker-compose.replication.yml の環境が起動中です。{RESET}")
        print("このスクリプトには docker-compose.yml (シングルサーバー) が必要です。\n")
        print("  docker compose -f docker-compose.replication.yml down")
        print("  docker compose up -d maharit-server\n")
        sys.exit(1)

    print(f"\n{RED}エラー: maharit-db-server コンテナが起動していません。{RESET}")
    print("以下のコマンドで起動してください:\n")
    print("  docker compose up -d maharit-server\n")
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="MaharitDB 大量データ性能ベンチマーク")
    parser.add_argument("--host",       default="localhost")
    parser.add_argument("--port",       type=int,   default=7687)
    parser.add_argument("--nodes",      type=int,   default=1000,
                        help="作成する Person ノード数 (デフォルト: 1000)")
    parser.add_argument("--edge-ratio", type=float, default=0.3,
                        help="エッジ数 = ノード数 × この比率 (デフォルト: 0.3)")
    parser.add_argument("--repeat",     type=int,   default=50,
                        help="連続読み取り回数 (デフォルト: 50)")
    parser.add_argument("--chunk-size", type=int,   default=100,
                        help="ストリーミングのチャンクサイズ (デフォルト: 100)")
    parser.add_argument("--parallel",   type=int,   default=8,
                        help="並行項目で使う接続数 (デフォルト: 8)")
    parser.add_argument("--no-parallel", action="store_true",
                        help="並行項目（複数接続のスループット・負荷中の応答時間）を省く")
    parser.add_argument("--no-cleanup", action="store_true",
                        help="ベンチ後にデータを削除しない")
    parser.add_argument("--scaling", action="store_true",
                        help="多ラベル大規模データでラベル索引・WHEREプッシュダウンの"
                             "スケーリング項目を追加実行する（例: --nodes 100000 --scaling）")
    parser.add_argument("--scaling-nodes", type=int, default=None,
                        help="スケーリング項目のノード数（省略時は --nodes と同じ）。"
                             "標準スイートは1件ずつ作成でネットワーク律速のため、"
                             "スケーリングだけ大きくしたい場合に指定する。")
    parser.add_argument("--output", default=None,
                        help="Markdown レポートの出力先ファイルパス (省略時は自動生成: "
                             "benchmark_reports/bench_YYYYMMDD_HHMMSS.md)")
    parser.add_argument("--output-json", default=None, metavar="PATH",
                        help="JSON 形式でも保存するパス "
                             "(例: benchmark_reports/baseline.json)。"
                             "perf_check.py によるベースライン比較に使用します。")
    args = parser.parse_args()
    check_docker_compose()

    print(f"\n{BOLD}MaharitDB 大量データ性能ベンチマーク{RESET}")
    print(f"接続先  : {args.host}:{args.port}")
    print(f"ノード数: {args.nodes:,}")
    print("─" * 80)

    try:
        client = MaharitClient(args.host, args.port, timeout=120.0)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    all_results: list[BenchResult] = []

    try:
        # Ping 確認
        resp = client.send({"type": "ping"})
        if resp.get("type") != "pong":
            print(f"{RED}サーバーが応答しません: {resp}{RESET}")
            sys.exit(1)
        print(f"  {GREEN}✓{RESET} サーバー接続 OK\n")

        # 事前クリーンアップ（残留データ除去）
        client.query("MATCH (n:BenchPerson) DETACH DELETE n")

        # ベンチマーク実行
        all_results.append(bench_create_nodes(client, args.nodes))
        all_results.append(bench_unwind_batch_create(client, args.nodes))
        all_results.append(bench_create_edges(client, args.nodes, args.edge_ratio))
        all_results.append(bench_full_scan(client))
        all_results.extend(bench_filter_queries(client, args.nodes))
        all_results.extend(bench_aggregation(client))
        all_results.extend(bench_traversal(client))
        all_results.append(bench_stream(client, args.chunk_size))
        all_results.append(bench_concurrent_reads(client, args.repeat))
        if not args.no_parallel:
            all_results.extend(bench_parallel(client, args.host, args.port, args.parallel))

        # スケーリング項目（多ラベル + 選択的ラベル + プッシュダウン）
        if args.scaling:
            scaling_n = args.scaling_nodes or args.nodes
            all_results.extend(bench_scaling_label_and_pushdown(client, scaling_n))

    finally:
        if not args.no_cleanup:
            cleanup(client)
            # スケーリングデータの取りこぼしを掃除（途中失敗時の保険）
            client.query("MATCH (n:ScaleFiller) DELETE n")
            client.query("MATCH (n:ScaleRare) DELETE n")
            client.query(f"MATCH (n:{TMP_LABEL}) DELETE n")
        client.close()

    print_summary(all_results)

    # レポート保存
    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    output_path = args.output or f"benchmark_reports/bench_{ts}.md"
    save_report(all_results, args, output_path)

    if args.output_json:
        save_json_report(all_results, args, args.output_json)


if __name__ == "__main__":
    main()
