#!/usr/bin/env python3
"""
MaharitDB 制約・インデックス E2E テスト

UNIQUE制約、NOT NULL制約、SHOW CONSTRAINTS、
フルテキストインデックスの動作を検証する。

使い方:
  python3 scripts/constraint_test.py
  python3 scripts/constraint_test.py --host localhost --port 7687
"""

import argparse
import json
import socket
import struct
import subprocess
import sys

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

    def send(self, request: dict) -> dict:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        return self._recv_one()

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

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
        errors.append(f"{name}{detail_str}")


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


def run_query(client: MaharitClient, query: str) -> dict:
    return client.send({"type": "query", "query": query})


def setup(client: MaharitClient):
    """テスト前にグラフと制約・インデックスをリセットする。"""
    run_query(client, "MATCH (n) DETACH DELETE n")
    # 既存の制約を削除（エラーは無視）
    for name in [
        "unique_product_sku",
        "unique_user_email",
        "notnull_product_name",
        "notnull_user_email",
    ]:
        run_query(client, f"DROP CONSTRAINT {name} IF EXISTS")
    # 既存のフルテキストインデックスを削除（エラーは無視）
    for name in ["ft_article_body", "ft_product_desc"]:
        run_query(client, f"DROP FULLTEXT INDEX {name} IF EXISTS")


def teardown(client: MaharitClient):
    """テスト後にグラフと制約・インデックスをリセットする。"""
    run_query(client, "MATCH (n) DETACH DELETE n")
    for name in [
        "unique_product_sku",
        "unique_user_email",
        "notnull_product_name",
        "notnull_user_email",
    ]:
        run_query(client, f"DROP CONSTRAINT {name} IF EXISTS")
    for name in ["ft_article_body", "ft_product_desc"]:
        run_query(client, f"DROP FULLTEXT INDEX {name} IF EXISTS")


# ── テストスイート ────────────────────────────────────────────────────────────

def test_unique_constraint(client: MaharitClient):
    section("UNIQUE 制約")

    # 制約の作成（構文: CREATE CONSTRAINT name FOR (var:Label) REQUIRE var.prop IS UNIQUE）
    resp = run_query(
        client,
        "CREATE CONSTRAINT unique_product_sku FOR (p:Product) REQUIRE p.sku IS UNIQUE",
    )
    check("UNIQUE 制約を作成", resp.get("type") == "result", str(resp))

    # 最初のノードは成功
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-001', name: 'Widget'})",
    )
    check("UNIQUE 制約がある状態で初回 CREATE は成功", resp.get("type") == "result", str(resp))

    # 同じ SKU で作成しようとするとエラー
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-001', name: 'Widget Duplicate'})",
    )
    check(
        "UNIQUE 制約違反でエラーが返る",
        resp.get("type") == "error",
        str(resp),
    )

    # 別の SKU は成功
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-002', name: 'Gadget'})",
    )
    check("別 SKU での CREATE は成功", resp.get("type") == "result", str(resp))

    # MERGE でも違反はエラー
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-001', name: 'Another'})",
    )
    check("UNIQUE 重複 CREATE の再確認", resp.get("type") == "error", str(resp))

    # 制約削除後は重複可能
    resp = run_query(client, "DROP CONSTRAINT unique_product_sku")
    check("UNIQUE 制約の削除", resp.get("type") == "result", str(resp))

    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-001', name: 'Widget Copy'})",
    )
    check("制約削除後の重複 CREATE は成功", resp.get("type") == "result", str(resp))


def test_not_null_constraint(client: MaharitClient):
    section("NOT NULL 制約")

    # 制約の作成（構文: CREATE CONSTRAINT name FOR (var:Label) REQUIRE var.prop IS NOT NULL）
    resp = run_query(
        client,
        "CREATE CONSTRAINT notnull_product_name FOR (p:Product) REQUIRE p.name IS NOT NULL",
    )
    check("NOT NULL 制約を作成", resp.get("type") == "result", str(resp))

    # name あり → 成功
    resp = run_query(
        client,
        "CREATE (:Product {name: 'Validating Widget', sku: 'SKU-NN-001'})",
    )
    check("NOT NULL 制約がある状態で name あり CREATE は成功", resp.get("type") == "result", str(resp))

    # name なし → エラー
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-NN-002'})",
    )
    check(
        "NOT NULL 制約違反（name なし）でエラーが返る",
        resp.get("type") == "error",
        str(resp),
    )

    # 制約削除
    resp = run_query(client, "DROP CONSTRAINT notnull_product_name")
    check("NOT NULL 制約の削除", resp.get("type") == "result", str(resp))

    # 削除後は name なしでも作成可能
    resp = run_query(
        client,
        "CREATE (:Product {sku: 'SKU-NN-003'})",
    )
    check("制約削除後の name なし CREATE は成功", resp.get("type") == "result", str(resp))


def test_show_constraints(client: MaharitClient):
    section("SHOW CONSTRAINTS")

    # 複数の制約を作成（構文: FOR (var:Label) REQUIRE var.prop IS ...）
    run_query(
        client,
        "CREATE CONSTRAINT unique_user_email FOR (u:User) REQUIRE u.email IS UNIQUE",
    )
    run_query(
        client,
        "CREATE CONSTRAINT notnull_user_email FOR (u:User) REQUIRE u.email IS NOT NULL",
    )

    resp = run_query(client, "SHOW CONSTRAINTS")
    check("SHOW CONSTRAINTS でエラーなし", resp.get("type") == "result", str(resp))

    rows = resp.get("rows", [])
    # 作成した2件が含まれているはず
    constraint_names = set()
    for row in rows:
        for v in row.values():
            if "unique_user_email" in str(v) or "notnull_user_email" in str(v):
                constraint_names.add(str(v))
    check(
        "SHOW CONSTRAINTS に作成した制約が含まれる",
        len(rows) >= 2,
        f"rows={len(rows)}",
    )

    # 削除後は消えている
    run_query(client, "DROP CONSTRAINT unique_user_email")
    run_query(client, "DROP CONSTRAINT notnull_user_email")

    resp = run_query(client, "SHOW CONSTRAINTS")
    rows_after = resp.get("rows", [])
    check(
        "DROP CONSTRAINT 後に件数が減る",
        len(rows_after) < len(rows) or len(rows_after) == 0,
        f"before={len(rows)}, after={len(rows_after)}",
    )


def test_fulltext_index(client: MaharitClient):
    section("フルテキストインデックス")

    # インデックスの作成（構文: CREATE FULLTEXT INDEX name FOR (var:Label) ON (var.prop)）
    resp = run_query(
        client,
        "CREATE FULLTEXT INDEX ft_article_body FOR (a:Article) ON (a.body)",
    )
    check("フルテキストインデックス作成", resp.get("type") == "result", str(resp))

    # テストデータ投入
    run_query(client, "CREATE (:Article {title: 'GraphDB intro', body: 'Graph databases are powerful for connected data'})")
    run_query(client, "CREATE (:Article {title: 'SQL vs Graph', body: 'Relational databases use tables while graph uses nodes'})")
    run_query(client, "CREATE (:Article {title: 'Performance tips', body: 'Indexing improves query performance significantly'})")

    # CONTAINS 検索
    resp = run_query(
        client,
        "MATCH (a:Article) WHERE a.body CONTAINS 'graph' RETURN a.title",
    )
    rows = resp.get("rows", [])
    check(
        "CONTAINS 'graph' で関連記事が返る",
        len(rows) >= 1,
        f"rows={len(rows)}",
    )

    # 別のキーワード
    resp = run_query(
        client,
        "MATCH (a:Article) WHERE a.body CONTAINS 'performance' RETURN a.title",
    )
    rows = resp.get("rows", [])
    check(
        "CONTAINS 'performance' で Performance 記事が返る",
        len(rows) >= 1,
        f"rows={len(rows)}",
    )

    # 存在しないキーワード
    resp = run_query(
        client,
        "MATCH (a:Article) WHERE a.body CONTAINS 'zyxwvuts' RETURN a.title",
    )
    rows = resp.get("rows", [])
    check(
        "CONTAINS で存在しないキーワードは 0 件",
        len(rows) == 0,
        f"rows={len(rows)}",
    )

    # インデックス削除
    resp = run_query(client, "DROP FULLTEXT INDEX ft_article_body")
    check("フルテキストインデックス削除", resp.get("type") == "result", str(resp))

    # 削除後も CONTAINS は動作する（フルスキャンにフォールバック）
    resp = run_query(
        client,
        "MATCH (a:Article) WHERE a.body CONTAINS 'graph' RETURN a.title",
    )
    check("インデックス削除後も CONTAINS は動作する", resp.get("type") == "result", str(resp))


def test_property_index(client: MaharitClient):
    section("プロパティインデックス (CREATE INDEX)")

    # インデックス作成
    resp = run_query(
        client,
        "CREATE INDEX ON :Employee(department)",
    )
    check("プロパティインデックス作成", resp.get("type") == "result", str(resp))

    # インデックスがあっても通常のクエリは動作する
    run_query(client, "CREATE (:Employee {name: 'Taro', department: 'Engineering'})")
    run_query(client, "CREATE (:Employee {name: 'Hanako', department: 'Marketing'})")
    run_query(client, "CREATE (:Employee {name: 'Jiro', department: 'Engineering'})")

    resp = run_query(
        client,
        "MATCH (e:Employee {department: 'Engineering'}) RETURN e.name",
    )
    rows = resp.get("rows", [])
    check(
        "インデックスあり状態でプロパティ検索が動作する",
        len(rows) == 2,
        f"rows={len(rows)}",
    )

    # SHOW INDEXES
    resp = run_query(client, "SHOW INDEXES")
    check("SHOW INDEXES でエラーなし", resp.get("type") == "result", str(resp))

    # インデックス削除
    resp = run_query(client, "DROP INDEX ON :Employee(department)")
    check("プロパティインデックス削除", resp.get("type") == "result", str(resp))

    # 削除後もクエリは動作する（フルスキャン）
    resp = run_query(
        client,
        "MATCH (e:Employee {department: 'Engineering'}) RETURN e.name",
    )
    rows = resp.get("rows", [])
    check(
        "インデックス削除後もプロパティ検索は動作する",
        len(rows) == 2,
        f"rows={len(rows)}",
    )


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
    parser = argparse.ArgumentParser(description="MaharitDB 制約・インデックス E2E テスト")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()
    check_docker_compose()

    print(f"\n{BOLD}MaharitDB 制約・インデックス E2E テスト{RESET}")
    print(f"接続先: {args.host}:{args.port}")
    print("─" * 50)

    try:
        client = MaharitClient(args.host, args.port)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    setup(client)

    try:
        test_unique_constraint(client)
        test_not_null_constraint(client)
        test_show_constraints(client)
        test_fulltext_index(client)
        test_property_index(client)
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
