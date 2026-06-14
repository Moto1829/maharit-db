#!/usr/bin/env python3
"""
MaharitDB Smoke Test
ローカルで起動中の maharit-db に対して各種操作を行い動作確認する。

使い方:
  python3 scripts/smoke_test.py
  python3 scripts/smoke_test.py --host localhost --port 7687
"""

import argparse
import os
import subprocess
import sys
import time

# scripts/ を import path に追加して lib モジュールを使えるようにする
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import (  # noqa: E402
    BOLD,
    CYAN,
    GREEN,
    RED,
    RESET,
    YELLOW,
    check,
    section,
    summarize,
)


def run_query(client: MaharitClient, query: str) -> dict:
    return client.send({"type": "query", "query": query})


# ── テストスイート ────────────────────────────────────────────────────────────

def test_ping(client: MaharitClient):
    section("Ping")
    resp = client.send({"type": "ping"})
    check("pong が返る", resp.get("type") == "pong", str(resp))


def test_stats(client: MaharitClient):
    section("Stats")
    resp = client.send({"type": "stats"})
    check("stats が返る", resp.get("type") == "stats", str(resp))
    check("nodes フィールドがある", "nodes" in resp)
    check("edges フィールドがある", "edges" in resp)


def test_create_nodes(client: MaharitClient):
    section("CREATE ノード")

    resp = run_query(client, "CREATE (a:Person {name: 'Alice', age: 30}) RETURN a")
    check("Alice 作成", resp.get("type") == "result", str(resp))

    resp = run_query(client, "CREATE (b:Person {name: 'Bob', age: 25}) RETURN b")
    check("Bob 作成", resp.get("type") == "result", str(resp))

    resp = run_query(client, "CREATE (c:Company {name: 'Acme'}) RETURN c")
    check("Acme 作成", resp.get("type") == "result", str(resp))


def test_create_edges(client: MaharitClient):
    section("CREATE エッジ")

    resp = run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
        "CREATE (a)-[:KNOWS]->(b) RETURN a, b",
    )
    check("KNOWS エッジ作成", resp.get("type") == "result", str(resp))

    resp = run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (c:Company {name: 'Acme'}) "
        "CREATE (a)-[:WORKS_AT]->(c) RETURN a, c",
    )
    check("WORKS_AT エッジ作成", resp.get("type") == "result", str(resp))


def test_match(client: MaharitClient):
    section("MATCH 検索")

    resp = run_query(client, "MATCH (n:Person) RETURN n")
    check("Person ノード取得", resp.get("type") == "result", str(resp))
    check("2件以上ある", len(resp.get("rows", [])) >= 2,
          f"rows={len(resp.get('rows', []))}")

    resp = run_query(client, "MATCH (n:Person) WHERE n.age > 26 RETURN n")
    check("WHERE age > 26 フィルタ", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    check("Alice のみ返る", len(rows) == 1, f"rows={len(rows)}")


def test_relationship_match(client: MaharitClient):
    section("MATCH リレーションシップ")

    resp = run_query(
        client,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b",
    )
    check("KNOWS トラバーサル", resp.get("type") == "result", str(resp))
    check("1件返る", len(resp.get("rows", [])) >= 1,
          f"rows={len(resp.get('rows', []))}")

    resp = run_query(
        client,
        "MATCH (a:Person)-[:WORKS_AT]->(c:Company) RETURN a.name, c.name",
    )
    check("WORKS_AT トラバーサル", resp.get("type") == "result", str(resp))


def test_set(client: MaharitClient):
    section("SET 更新")

    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Bob'}) SET n.age = 26 RETURN n",
    )
    check("Bob の age を更新", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Bob'}) RETURN n.age")
    rows = resp.get("rows", [])
    age_val = rows[0].get("n.age") if rows else None
    check("更新後の age が 26", age_val == "26", f"age={age_val}")


def test_delete(client: MaharitClient):
    section("DELETE 削除")

    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Bob'}) DETACH DELETE n",
    )
    check("Bob を削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Bob'}) RETURN n")
    check("削除後は見つからない", len(resp.get("rows", [])) == 0,
          f"rows={len(resp.get('rows', []))}")


def test_transaction_commit(client: MaharitClient):
    section("トランザクション COMMIT")

    resp = client.send({"type": "begin"})
    check("BEGIN", resp.get("type") == "transactionBegun", str(resp))
    tx_id = resp.get("txId")

    run_query(client, "CREATE (t:TxTest {val: 'committed'})")

    resp = client.send({"type": "commit", "txId": tx_id})
    check("COMMIT", resp.get("type") == "committed", str(resp))

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    check("COMMIT 後にデータが残る", len(resp.get("rows", [])) >= 1,
          f"rows={len(resp.get('rows', []))}")


def test_transaction_rollback(client: MaharitClient):
    section("トランザクション ROLLBACK")

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    before = len(resp.get("rows", []))

    resp = client.send({"type": "begin"})
    check("BEGIN", resp.get("type") == "transactionBegun", str(resp))
    tx_id = resp.get("txId")

    client.send({"type": "query", "query": "CREATE (t:TxTest {val: 'will_rollback'})", "txId": tx_id})

    resp = client.send({"type": "rollback", "txId": tx_id})
    check("ROLLBACK", resp.get("type") == "rolledBack", str(resp))

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    after = len(resp.get("rows", []))
    check("ROLLBACK 後に件数が変わらない", after == before,
          f"before={before}, after={after}")


def test_stream_query(client: MaharitClient):
    section("ストリーミングクエリ")

    # 複数ノードを作成してからストリームで取得
    run_query(client, "CREATE (:StreamTest {i: 1})")
    run_query(client, "CREATE (:StreamTest {i: 2})")
    run_query(client, "CREATE (:StreamTest {i: 3})")

    messages = client.send({
        "type": "streamQuery",
        "query": "MATCH (n:StreamTest) RETURN n",
        "chunkSize": 2,
    })

    types = [m.get("type") for m in messages]
    check("streamStart が含まれる", "streamStart" in types, str(types))
    check("streamEnd が含まれる", "streamEnd" in types, str(types))

    chunks = [m for m in messages if m.get("type") == "streamChunk"]
    total_rows = sum(len(c.get("rows", [])) for c in chunks)
    check("3件のデータが返る", total_rows >= 3, f"total_rows={total_rows}")


def test_error_handling(client: MaharitClient):
    section("エラーハンドリング")

    resp = run_query(client, "INVALID QUERY !!!")
    check("不正クエリでエラーが返る", resp.get("type") == "error", str(resp))


def test_cleanup(client: MaharitClient):
    section("クリーンアップ")

    resp = run_query(client, "MATCH (n) DETACH DELETE n")
    check("全ノード削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n) RETURN n")
    check("全ノード削除済み", len(resp.get("rows", [])) == 0,
          f"残り={len(resp.get('rows', []))}")


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
    parser = argparse.ArgumentParser(description="MaharitDB Smoke Test")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()
    check_docker_compose()

    print(f"\n{BOLD}MaharitDB Smoke Test{RESET}")
    print(f"接続先: {args.host}:{args.port}")
    print("─" * 40)

    try:
        client = MaharitClient(args.host, args.port)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    # テスト開始前にDB全体をリセットして冪等性を確保
    run_query(client, "MATCH (n) DETACH DELETE n")

    try:
        test_ping(client)
        test_stats(client)
        test_create_nodes(client)
        test_create_edges(client)
        test_match(client)
        test_relationship_match(client)
        test_set(client)
        test_delete(client)
        test_transaction_commit(client)
        test_transaction_rollback(client)
        test_stream_query(client)
        test_error_handling(client)
        test_cleanup(client)
    finally:
        client.close()

    sys.exit(summarize())


if __name__ == "__main__":
    main()
