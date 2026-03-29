#!/usr/bin/env python3
"""
MaharitDB 大量データ性能ベンチマーク
起動中の maharit-db サーバーに対して大量データを投入し、各種クエリの性能を計測する。

使い方:
  python3 scripts/benchmark.py
  python3 scripts/benchmark.py --nodes 10000 --host localhost --port 7687
  python3 scripts/benchmark.py --nodes 1000 --no-cleanup
  python3 scripts/benchmark.py --output reports/bench_20260320.md

分析:
  ベンチ完了後、Claude Code で /analyze-benchmark を実行すると
  最新レポートを自動で読み込んで考察を追記します。
"""

import argparse
import json
import os
import random
import socket
import struct
import subprocess
import sys
import time
from datetime import datetime, timezone

# ── ANSI カラー ───────────────────────────────────────────────────────────────
GREEN  = "\033[92m"
YELLOW = "\033[93m"
RED    = "\033[91m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
RESET  = "\033[0m"

NAMES   = ["Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi",
           "Ivan", "Judy", "Karl", "Laura", "Mallory", "Niaj", "Olivia", "Peggy"]
CITIES  = ["Tokyo", "Osaka", "Nagoya", "Sapporo", "Fukuoka", "Kyoto",
           "Yokohama", "Kobe", "Sendai", "Hiroshima"]
SKILLS  = ["Rust", "Python", "Go", "Java", "TypeScript", "C++", "Haskell",
           "Scala", "Elixir", "Ruby"]


# ── プロトコル実装 ─────────────────────────────────────────────────────────────

class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 60.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict | list[dict]:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        if request.get("type") == "streamQuery":
            return self._recv_stream()
        return self._recv_one()

    def query(self, cypher: str) -> dict:
        return self.send({"type": "query", "query": cypher})

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        return json.loads(self._recv_exactly(length))

    def _recv_stream(self) -> list[dict]:
        messages = []
        while True:
            msg = self._recv_one()
            messages.append(msg)
            if msg.get("type") in ("streamEnd", "error"):
                break
        return messages

    def _recv_exactly(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("Connection closed by server")
            buf += chunk
        return buf

    def close(self):
        try:
            self.send({"type": "disconnect"})
        except Exception:
            pass
        self.sock.close()


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
    start = time.perf_counter()
    resp  = client.query("MATCH (n:BenchPerson) RETURN n")
    elapsed = time.perf_counter() - start
    count = len(resp.get("rows", []))
    r = BenchResult("MATCH full scan (BenchPerson)", count, elapsed)
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
        start   = time.perf_counter()
        resp    = client.query(cypher)
        elapsed = time.perf_counter() - start
        count   = len(resp.get("rows", []))
        r = BenchResult(f"MATCH {label}", count, elapsed)
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
        start   = time.perf_counter()
        resp    = client.query(cypher)
        elapsed = time.perf_counter() - start
        count   = len(resp.get("rows", []))
        r = BenchResult(f"AGG {label}", count, elapsed)
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
    ]

    for label, cypher in trav_queries:
        start   = time.perf_counter()
        resp    = client.query(cypher)
        elapsed = time.perf_counter() - start
        count   = len(resp.get("rows", []))
        r = BenchResult(f"TRAV {label}", count, elapsed)
        r.print()
        results.append(r)

    return results


def bench_stream(client: MaharitClient, chunk_size: int = 100) -> BenchResult:
    """ストリーミングで全ノードを取得する"""
    section(f"ストリーミング (chunkSize={chunk_size})")
    start    = time.perf_counter()
    messages = client.send({
        "type":      "streamQuery",
        "query":     "MATCH (n:BenchPerson) RETURN n",
        "chunkSize": chunk_size,
    })
    elapsed = time.perf_counter() - start
    chunks     = [m for m in messages if m.get("type") == "streamChunk"]
    total_rows = sum(len(c.get("rows", [])) for c in chunks)
    r = BenchResult(f"STREAM MATCH (chunk={chunk_size})", total_rows, elapsed)
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

    payload = {
        "timestamp":  now,
        "node_count": args.nodes,
        "host":       args.host,
        "port":       args.port,
        "results":    results,
    }

    os.makedirs(os.path.dirname(output_path) if os.path.dirname(output_path) else ".", exist_ok=True)
    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
        f.write("\n")

    print(f"  {GREEN}✓{RESET} JSON レポートを保存しました: {BOLD}{output_path}{RESET}")


# ── エントリポイント ──────────────────────────────────────────────────────────

def check_docker_compose() -> None:
    """docker-compose.yml の maharit-db-server が起動中か確認する。"""
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
    parser.add_argument("--no-cleanup", action="store_true",
                        help="ベンチ後にデータを削除しない")
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

    finally:
        if not args.no_cleanup:
            cleanup(client)
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
