#!/usr/bin/env python3
"""
MaharitDB クエリ機能 E2E テスト

smoke_test.py がカバーしていない詳細なクエリ機能を検証する。

使い方:
  python3 scripts/query_feature_test.py
  python3 scripts/query_feature_test.py --host localhost --port 7687
"""

import argparse
import json
import socket
import struct
import subprocess
import sys
import time

# ── ANSI カラー ──────────────────────────────────────────────────────────────
GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
RESET  = "\033[0m"


# ── プロトコル実装（4バイト長プレフィックス + JSON） ─────────────────────────

class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 10.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict | list[dict]:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)

        resp_type = request.get("type")
        if resp_type == "streamQuery":
            return self._recv_stream()
        return self._recv_one()

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

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


# ── テストヘルパー ────────────────────────────────────────────────────────────

passed = 0
failed = 0
errors = []


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  {GREEN}✓{RESET} {name}")
    else:
        failed += 1
        detail_str = f" — {detail}" if detail else ""
        print(f"  {RED}✗{RESET} {name}{detail_str}")
        errors.append(f"{name}: {detail_str}")


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


def run_query(client: MaharitClient, query: str) -> dict:
    return client.send({"type": "query", "query": query})


def setup(client: MaharitClient):
    """テスト開始前にDBを初期化し、テストデータを投入する。"""
    run_query(client, "MATCH (n) DETACH DELETE n")
    # ベースデータ投入
    run_query(client, "CREATE (:Person {name: 'Alice', age: 30, city: 'Tokyo'})")
    run_query(client, "CREATE (:Person {name: 'Bob', age: 25, city: 'Osaka'})")
    run_query(client, "CREATE (:Person {name: 'Charlie', age: 35, city: 'Tokyo'})")
    run_query(client, "CREATE (:Person {name: 'Dave', age: 20, city: 'Kyoto'})")
    run_query(client, "CREATE (:Company {name: 'Acme', industry: 'Tech'})")
    run_query(client, "CREATE (:Company {name: 'Globex', industry: 'Finance'})")
    # エッジ
    run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
        "CREATE (a)-[:KNOWS {since: 2020}]->(b)",
    )
    run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (c:Person {name: 'Charlie'}) "
        "CREATE (a)-[:KNOWS {since: 2018}]->(c)",
    )
    run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (co:Company {name: 'Acme'}) "
        "CREATE (a)-[:WORKS_AT]->(co)",
    )
    run_query(
        client,
        "MATCH (b:Person {name: 'Bob'}), (co:Company {name: 'Globex'}) "
        "CREATE (b)-[:WORKS_AT]->(co)",
    )


def teardown(client: MaharitClient):
    run_query(client, "MATCH (n) DETACH DELETE n")


# ── テストスイート ────────────────────────────────────────────────────────────

def strip_quotes(v: str) -> str:
    """サーバーが文字列を JSON エンコード済みで返す場合（"Alice" → Alice）の正規化。"""
    if v and v.startswith('"') and v.endswith('"'):
        return v[1:-1]
    return v


def test_where_filters(client: MaharitClient):
    section("WHERE フィルタ")

    # 比較演算子（数値）
    resp = run_query(client, "MATCH (n:Person) WHERE n.age > 28 RETURN n.name")
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE age > 28 で Alice と Charlie が返る",
        names >= {"Alice", "Charlie"},
        f"names={names}",
    )
    check(
        "WHERE age > 28 で Bob と Dave が除外される",
        "Bob" not in names and "Dave" not in names,
        f"names={names}",
    )

    # 比較演算子（文字列）
    resp = run_query(client, "MATCH (n:Person) WHERE n.city = 'Tokyo' RETURN n.name")
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE city = 'Tokyo' で Alice と Charlie が返る",
        names == {"Alice", "Charlie"},
        f"names={names}",
    )

    # AND 論理演算子
    resp = run_query(
        client,
        "MATCH (n:Person) WHERE n.age > 24 AND n.city = 'Tokyo' RETURN n.name",
    )
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE age > 24 AND city = 'Tokyo'",
        names == {"Alice", "Charlie"},
        f"names={names}",
    )

    # OR 論理演算子
    resp = run_query(
        client,
        "MATCH (n:Person) WHERE n.city = 'Osaka' OR n.city = 'Kyoto' RETURN n.name",
    )
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE city = 'Osaka' OR city = 'Kyoto'",
        names == {"Bob", "Dave"},
        f"names={names}",
    )

    # NOT 論理演算子
    resp = run_query(
        client,
        "MATCH (n:Person) WHERE NOT n.city = 'Tokyo' RETURN n.name",
    )
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE NOT city = 'Tokyo' で非Tokyoが返る",
        "Alice" not in names and "Charlie" not in names,
        f"names={names}",
    )

    # <= 演算子
    resp = run_query(client, "MATCH (n:Person) WHERE n.age <= 25 RETURN n.name")
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("n.name", "")) for r in rows}
    check(
        "WHERE age <= 25 で Bob と Dave が返る",
        names == {"Bob", "Dave"},
        f"names={names}",
    )


def test_return_expressions(client: MaharitClient):
    section("RETURN 式")

    # プロパティアクセス
    resp = run_query(client, "MATCH (n:Person {name: 'Alice'}) RETURN n.name, n.age")
    rows = resp.get("rows", [])
    check("プロパティアクセスで rows が返る", len(rows) == 1, f"rows={len(rows)}")
    check(
        "n.name が Alice",
        strip_quotes(rows[0].get("n.name", "")) == "Alice" if rows else False,
        str(rows),
    )
    check(
        "n.age が 30",
        rows[0].get("n.age") == "30" if rows else False,
        str(rows),
    )

    # エイリアス
    resp = run_query(client, "MATCH (n:Person {name: 'Alice'}) RETURN n.name AS person_name, n.age AS person_age")
    rows = resp.get("rows", [])
    check("エイリアスで rows が返る", len(rows) == 1, f"rows={len(rows)}")
    check(
        "person_name エイリアスが使える",
        strip_quotes(rows[0].get("person_name", "")) == "Alice" if rows else False,
        str(rows),
    )

    # RETURN DISTINCT
    resp = run_query(client, "MATCH (n:Person) RETURN DISTINCT n.city")
    rows = resp.get("rows", [])
    cities = {strip_quotes(r.get("n.city", "")) for r in rows}
    check(
        "DISTINCT city が重複なしで返る",
        len(cities) == 3 and cities == {"Tokyo", "Osaka", "Kyoto"},
        f"cities={cities}",
    )


def test_aggregation(client: MaharitClient):
    section("集計関数")

    # COUNT(*)
    resp = run_query(client, "MATCH (n:Person) RETURN COUNT(*) AS cnt")
    rows = resp.get("rows", [])
    cnt = rows[0].get("cnt") if rows else None
    check("COUNT(*) が 4 を返す", cnt == "4", f"cnt={cnt}")

    # COUNT(n)
    resp = run_query(client, "MATCH (n:Person) RETURN COUNT(n) AS cnt")
    rows = resp.get("rows", [])
    cnt = rows[0].get("cnt") if rows else None
    check("COUNT(n) が 4 を返す", cnt == "4", f"cnt={cnt}")

    # SUM
    resp = run_query(client, "MATCH (n:Person) RETURN SUM(n.age) AS total")
    rows = resp.get("rows", [])
    total = rows[0].get("total") if rows else None
    # 30 + 25 + 35 + 20 = 110
    check("SUM(n.age) が 110", total == "110", f"total={total}")

    # AVG
    resp = run_query(client, "MATCH (n:Person) RETURN AVG(n.age) AS avg_age")
    rows = resp.get("rows", [])
    avg_age = rows[0].get("avg_age") if rows else None
    # 110 / 4 = 27.5
    check("AVG(n.age) が 27.5", avg_age is not None and float(avg_age) == 27.5, f"avg_age={avg_age}")

    # MIN / MAX
    resp = run_query(client, "MATCH (n:Person) RETURN MIN(n.age) AS min_age, MAX(n.age) AS max_age")
    rows = resp.get("rows", [])
    min_age = rows[0].get("min_age") if rows else None
    max_age = rows[0].get("max_age") if rows else None
    check("MIN(n.age) が 20", min_age == "20", f"min_age={min_age}")
    check("MAX(n.age) が 35", max_age == "35", f"max_age={max_age}")

    # COLLECT
    resp = run_query(client, "MATCH (n:Person) RETURN COLLECT(n.name) AS names")
    rows = resp.get("rows", [])
    names_raw = rows[0].get("names") if rows else None
    check("COLLECT(n.name) が非空のリストを返す", names_raw is not None, f"names={names_raw}")


def test_order_by_limit_skip(client: MaharitClient):
    section("ORDER BY / LIMIT / SKIP")

    # ORDER BY ASC
    resp = run_query(client, "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC")
    rows = resp.get("rows", [])
    ages = [r.get("n.age") for r in rows]
    check("ORDER BY age ASC の結果件数が 4", len(ages) == 4, f"ages={ages}")
    check(
        "ORDER BY age ASC で昇順になっている",
        ages == sorted(ages),
        f"ages={ages}",
    )

    # ORDER BY DESC
    resp = run_query(client, "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC")
    rows = resp.get("rows", [])
    ages = [r.get("n.age") for r in rows]
    check(
        "ORDER BY age DESC で降順になっている",
        ages == sorted(ages, reverse=True),
        f"ages={ages}",
    )

    # LIMIT
    resp = run_query(client, "MATCH (n:Person) RETURN n.name LIMIT 2")
    rows = resp.get("rows", [])
    check("LIMIT 2 で 2 件返る", len(rows) == 2, f"rows={len(rows)}")

    # SKIP
    resp = run_query(client, "MATCH (n:Person) RETURN n.name ORDER BY n.name SKIP 2")
    rows = resp.get("rows", [])
    check("SKIP 2 で 2 件返る", len(rows) == 2, f"rows={len(rows)}")

    # SKIP + LIMIT
    resp = run_query(client, "MATCH (n:Person) RETURN n.name ORDER BY n.age ASC SKIP 1 LIMIT 2")
    rows = resp.get("rows", [])
    check("SKIP 1 LIMIT 2 で 2 件返る", len(rows) == 2, f"rows={len(rows)}")


def test_pattern_matching(client: MaharitClient):
    section("パターンマッチング（ノード→エッジ→ノード）")

    # 1ホップ KNOWS
    resp = run_query(
        client,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name AS from, b.name AS to",
    )
    rows = resp.get("rows", [])
    check("KNOWS パターンで 2 件返る", len(rows) == 2, f"rows={len(rows)}")
    froms = {strip_quotes(r.get("from", "")) for r in rows}
    check("送信元が Alice", froms == {"Alice"}, f"froms={froms}")

    # 無向エッジ
    resp = run_query(
        client,
        "MATCH (a:Person)-[:KNOWS]-(b:Person) RETURN a.name AS n1, b.name AS n2",
    )
    rows = resp.get("rows", [])
    # Alice->Bob, Alice->Charlie, Bob->Alice, Charlie->Alice の 4 件
    check("無向 KNOWS パターンで 4 件返る", len(rows) == 4, f"rows={len(rows)}")

    # WORKS_AT 経由で会社名取得
    resp = run_query(
        client,
        "MATCH (p:Person)-[:WORKS_AT]->(c:Company) RETURN p.name AS person, c.name AS company",
    )
    rows = resp.get("rows", [])
    check("WORKS_AT パターンで 2 件返る", len(rows) == 2, f"rows={len(rows)}")

    # エッジプロパティでフィルタ
    resp = run_query(
        client,
        "MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE r.since = 2020 RETURN a.name, b.name",
    )
    rows = resp.get("rows", [])
    check("エッジプロパティフィルタ (since=2020) で 1 件返る", len(rows) == 1, f"rows={len(rows)}")

    # 変数長パス（2ホップ以内）
    resp = run_query(
        client,
        "MATCH (a:Person {name: 'Alice'})-[:KNOWS*1..2]->(b) RETURN b.name",
    )
    rows = resp.get("rows", [])
    check("変数長パス *1..2 で結果が返る", len(rows) >= 2, f"rows={len(rows)}")


def test_optional_match(client: MaharitClient):
    section("OPTIONAL MATCH")

    resp = run_query(
        client,
        "MATCH (p:Person) "
        "OPTIONAL MATCH (p)-[:MANAGES]->(q:Person) "
        "RETURN p.name, q.name AS managed",
    )
    rows = resp.get("rows", [])
    check("OPTIONAL MATCH で全 Person が返る", len(rows) == 4, f"rows={len(rows)}")

    # 存在するエッジの OPTIONAL MATCH
    resp = run_query(
        client,
        "MATCH (p:Person {name: 'Alice'}) "
        "OPTIONAL MATCH (p)-[:KNOWS]->(q:Person) "
        "RETURN p.name, q.name AS friend",
    )
    rows = resp.get("rows", [])
    check("OPTIONAL MATCH で存在するエッジが返る", len(rows) >= 1, f"rows={len(rows)}")


def test_match_set(client: MaharitClient):
    section("MATCH + SET（プロパティ更新）")

    # プロパティ更新
    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Dave'}) SET n.age = 21 RETURN n.age",
    )
    check("SET で age を 21 に更新", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Dave'}) RETURN n.age")
    rows = resp.get("rows", [])
    check(
        "更新後の age が 21",
        rows[0].get("n.age") == "21" if rows else False,
        str(rows),
    )

    # 新プロパティ追加
    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Dave'}) SET n.email = 'dave@example.com' RETURN n.email",
    )
    check("SET で新プロパティ email を追加", resp.get("type") == "result", str(resp))

    # ラベル追加
    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Dave'}) SET n:VIP RETURN n",
    )
    check("SET でラベル VIP を追加", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:VIP) RETURN n.name")
    rows = resp.get("rows", [])
    check("VIP ラベルで Dave が取得できる", strip_quotes(rows[0].get("n.name", "")) == "Dave" if rows else False, str(rows))


def test_detach_delete(client: MaharitClient):
    section("MATCH + DETACH DELETE")

    # エッジを持つノードを DETACH DELETE
    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Bob'}) DETACH DELETE n",
    )
    check("DETACH DELETE で Bob を削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Bob'}) RETURN n")
    check("削除後は Bob が見つからない", len(resp.get("rows", [])) == 0, str(resp))

    # Bob の WORKS_AT エッジも消えているはず
    resp = run_query(
        client,
        "MATCH (p:Person)-[:WORKS_AT]->(c:Company {name: 'Globex'}) RETURN p.name",
    )
    check("DETACH DELETE 後に Globex への WORKS_AT エッジが消える", len(resp.get("rows", [])) == 0, str(resp))


def test_merge(client: MaharitClient):
    section("MERGE（upsert）")

    # 存在しない場合は作成
    resp = run_query(
        client,
        "MERGE (n:Person {name: 'Eve'}) RETURN n.name",
    )
    check("MERGE で Eve を作成", resp.get("type") == "result", str(resp))

    # 再度 MERGE しても重複しない
    resp = run_query(
        client,
        "MERGE (n:Person {name: 'Eve'}) RETURN n.name",
    )
    check("MERGE 2 回目でエラーなし", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Eve'}) RETURN n")
    check("MERGE 後に Eve が 1 件だけ存在する", len(resp.get("rows", [])) == 1, str(resp))

    # ON CREATE SET
    resp = run_query(
        client,
        "MERGE (n:Person {name: 'Frank'}) ON CREATE SET n.age = 28 RETURN n.name, n.age",
    )
    check("MERGE ON CREATE SET で Frank を作成", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    check(
        "ON CREATE SET で age が 28",
        rows[0].get("n.age") == "28" if rows else False,
        str(rows),
    )

    # ON MATCH SET
    resp = run_query(
        client,
        "MERGE (n:Person {name: 'Frank'}) ON MATCH SET n.age = 29 RETURN n.age",
    )
    check("MERGE ON MATCH SET で Frank の age を更新", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    check(
        "ON MATCH SET で age が 29",
        rows[0].get("n.age") == "29" if rows else False,
        str(rows),
    )


def test_unwind(client: MaharitClient):
    section("UNWIND（リスト展開）")

    # リストリテラルの展開
    resp = run_query(
        client,
        "UNWIND [1, 2, 3] AS x RETURN x",
    )
    rows = resp.get("rows", [])
    check("UNWIND [1,2,3] で 3 件返る", len(rows) == 3, f"rows={len(rows)}")
    xs = [r.get("x") for r in rows]
    check("UNWIND の値が 1, 2, 3", set(xs) == {"1", "2", "3"}, f"xs={xs}")

    # UNWIND + CREATE
    resp = run_query(
        client,
        "UNWIND ['Product A', 'Product B'] AS name CREATE (:Product {name: name}) RETURN name",
    )
    check("UNWIND + CREATE でエラーなし", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Product) RETURN n.name")
    rows = resp.get("rows", [])
    check("UNWIND + CREATE で 2 件作成された", len(rows) == 2, f"rows={len(rows)}")


def test_with_pipeline(client: MaharitClient):
    section("WITH（パイプライン）")

    # WITH で中間結果をフィルタ
    resp = run_query(
        client,
        "MATCH (n:Person) WITH n WHERE n.age > 25 RETURN n.name",
    )
    rows = resp.get("rows", [])
    names = {r.get("n.name") for r in rows}
    check(
        "WITH + WHERE フィルタで age > 25 の Person が返る",
        "Dave" not in names and "Bob" not in names,
        f"names={names}",
    )

    # WITH + COUNT で集計してから RETURN
    resp = run_query(
        client,
        "MATCH (n:Person) WITH n.city AS city, COUNT(n) AS cnt RETURN city, cnt ORDER BY cnt DESC",
    )
    rows = resp.get("rows", [])
    check("WITH 集計で rows が返る", len(rows) >= 1, f"rows={rows}")
    # Tokyo: Alice + Charlie = 2（cnt が正しく集計されているか確認）
    tokyo_row = next(
        (r for r in rows if strip_quotes(r.get("city", "")) == "Tokyo"), None
    )
    # NOTE: WITH + 集計関数のグループ化にバグがある場合 cnt が null になる
    tokyo_cnt = tokyo_row.get("cnt") if tokyo_row else None
    check(
        "Tokyo の cnt が 2（WITH グループ集計）",
        tokyo_cnt == "2",
        f"tokyo_row={tokyo_row} (cnt={tokyo_cnt}; null の場合は WITH 集計バグ: todo/bug 参照)",
    )

    # WITH LIMIT
    resp = run_query(
        client,
        "MATCH (n:Person) WITH n LIMIT 2 RETURN n.name",
    )
    rows = resp.get("rows", [])
    check("WITH LIMIT 2 で 2 件返る", len(rows) == 2, f"rows={len(rows)}")


def test_union(client: MaharitClient):
    section("UNION")

    # UNION テスト用のデータを確保（test_detach_delete で Bob が削除されているため再作成）
    run_query(client, "MERGE (:Person {name: 'Bob', age: 25, city: 'Osaka'})")

    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Alice'}) RETURN n.name AS name "
        "UNION "
        "MATCH (n:Person {name: 'Bob'}) RETURN n.name AS name",
    )
    check("UNION でエラーなし", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    names = {strip_quotes(r.get("name", "")) for r in rows}
    check("UNION で Alice と Bob が返る", names == {"Alice", "Bob"}, f"names={names}")

    # UNION ALL（重複含む）
    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Alice'}) RETURN n.name AS name "
        "UNION ALL "
        "MATCH (n:Person {name: 'Alice'}) RETURN n.name AS name",
    )
    check("UNION ALL でエラーなし", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    check("UNION ALL で重複を含む 2 件返る", len(rows) == 2, f"rows={len(rows)}")


def test_explain_profile(client: MaharitClient):
    section("EXPLAIN / PROFILE")

    resp = run_query(client, "EXPLAIN MATCH (n:Person) RETURN n")
    check("EXPLAIN でエラーなし", resp.get("type") in ("result", "plan"), str(resp))

    resp = run_query(client, "PROFILE MATCH (n:Person) RETURN n")
    check("PROFILE でエラーなし", resp.get("type") in ("result", "profile", "stats"), str(resp))


# ── エントリポイント ──────────────────────────────────────────────────────────

def check_docker_compose() -> None:
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
        print("  docker compose -f docker-compose.replication.yml down")
        print("  docker compose up -d maharit-server\n")
        sys.exit(1)

    print(f"\n{RED}エラー: maharit-db-server コンテナが起動していません。{RESET}")
    print("  docker compose up -d maharit-server\n")
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="MaharitDB クエリ機能 E2E テスト")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()
    check_docker_compose()

    print(f"\n{BOLD}MaharitDB クエリ機能 E2E テスト{RESET}")
    print(f"接続先: {args.host}:{args.port}")
    print("─" * 50)

    try:
        client = MaharitClient(args.host, args.port)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    setup(client)

    try:
        test_where_filters(client)
        test_return_expressions(client)
        test_aggregation(client)
        test_order_by_limit_skip(client)
        test_pattern_matching(client)
        test_optional_match(client)
        test_match_set(client)
        test_detach_delete(client)
        test_merge(client)
        test_unwind(client)
        test_with_pipeline(client)
        test_union(client)
        test_explain_profile(client)
    finally:
        teardown(client)
        client.close()

    print(f"\n{'─' * 50}")
    print(f"{BOLD}結果: {GREEN}{passed} passed{RESET}", end="")
    if failed:
        print(f", {RED}{failed} failed{RESET}")
        print(f"\n失敗したテスト:")
        for e in errors:
            print(f"  {RED}✗{RESET} {e}")
    else:
        print()

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
