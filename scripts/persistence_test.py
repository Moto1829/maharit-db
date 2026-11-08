#!/usr/bin/env python3
"""
MaharitDB 永続化ラウンドトリップテスト

サーバーを起動してデータを書き込み、停止後に再起動してデータが
正しく復元されることを検証する。

使い方:
  python3 scripts/persistence_test.py
  python3 scripts/persistence_test.py --binary ./target/release/maharit
  python3 scripts/persistence_test.py --port 7699 --data /tmp/my_test.db
"""

import argparse
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import check, section, summarize  # noqa: E402


def find_binary() -> str:
    """バイナリを探す: release > debug の順"""
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(root, "target", "release", "maharit"),
        os.path.join(root, "target", "debug", "maharit"),
    ]
    for path in candidates:
        if os.path.isfile(path):
            return path
    raise FileNotFoundError(
        "maharit バイナリが見つかりません。先に `cargo build` を実行してください。"
    )


def start_server(binary: str, port: int, data_path: str) -> subprocess.Popen:
    """サーバープロセスを起動して返す。"""
    proc = subprocess.Popen(
        [binary, "server", "--port", str(port), "--data", data_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return proc


def wait_for_server(host: str, port: int, timeout: float = 10.0) -> bool:
    """サーバーが接続を受け付けるまで待機する。"""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            c = MaharitClient(host, port, timeout=1.0)
            if c.ping():
                c.close()
                return True
            c.close()
        except Exception:
            pass
        time.sleep(0.1)
    return False


def stop_server(proc: subprocess.Popen, sig: signal.Signals, wait_secs: float = 5.0) -> bool:
    """シグナルを送りプロセス終了を待つ。タイムアウト内に終了すれば True。"""
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return True  # already gone

    try:
        proc.wait(timeout=wait_secs)
        return True
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        return False


# ── テストスイート ────────────────────────────────────────────────────────────

def test_sigterm_roundtrip(binary: str, port: int, data_path: str):
    """正常終了（SIGTERM）後の再起動でデータが復元されること。"""
    section("SIGTERM ラウンドトリップ")

    # ── フェーズ1: 書き込み ────────────────────────────────────────────────
    proc = start_server(binary, port, data_path)
    ready = wait_for_server("127.0.0.1", port)
    check("サーバー起動", ready)
    if not ready:
        proc.kill(); proc.wait()
        return

    client = MaharitClient("127.0.0.1", port)

    # ノード・エッジ・プロパティを書き込む
    client.query("CREATE (n:Person {name: 'Alice', age: 30})")
    client.query("CREATE (n:Person {name: 'Bob', age: 25})")
    client.query("CREATE (n:City {name: 'Tokyo'})")
    client.query(
        "MATCH (a:Person {name: 'Alice'}), (c:City {name: 'Tokyo'}) "
        "CREATE (a)-[:LIVES_IN]->(c)"
    )

    # 100件の中規模データ
    for i in range(100):
        client.query(f"CREATE (n:Item {{idx: {i}, label: 'item-{i}'}})")

    # 書き込み確認
    resp = client.query("MATCH (n:Person) RETURN n.name")
    before_count = len(resp.get("rows", []))
    check("書き込み前: Person ノード2件", before_count == 2, f"count={before_count}")

    resp = client.query("MATCH (n:Item) RETURN n.idx")
    item_count = len(resp.get("rows", []))
    check("書き込み前: Item ノード100件", item_count == 100, f"count={item_count}")

    client.close()

    # ── SIGTERM で正常終了 ────────────────────────────────────────────────
    clean_exit = stop_server(proc, signal.SIGTERM)
    check("SIGTERM で正常終了", clean_exit)

    check("データファイルが生成された", os.path.isfile(data_path))

    # ── フェーズ2: 再起動 + 読み取り ──────────────────────────────────────
    proc2 = start_server(binary, port, data_path)
    ready2 = wait_for_server("127.0.0.1", port)
    check("再起動後サーバー起動", ready2)
    if not ready2:
        proc2.kill(); proc2.wait()
        return

    client2 = MaharitClient("127.0.0.1", port)

    resp = client2.query("MATCH (n:Person) RETURN n.name")
    rows = resp.get("rows", [])
    check("Person ノード2件が復元", len(rows) == 2, f"count={len(rows)}")

    names = {r.get("n.name", "").strip('"') for r in rows}
    check("Alice が復元", "Alice" in names, f"names={names}")
    check("Bob が復元", "Bob" in names, f"names={names}")

    resp = client2.query("MATCH (n:City {name: 'Tokyo'}) RETURN n.name")
    check("City ノードが復元", len(resp.get("rows", [])) == 1)

    resp = client2.query("MATCH (a:Person)-[:LIVES_IN]->(c:City) RETURN c.name")
    check("エッジ（LIVES_IN）が復元", len(resp.get("rows", [])) >= 1)

    resp = client2.query("MATCH (n:Person {name: 'Alice'}) RETURN n.age")
    rows = resp.get("rows", [])
    check(
        "Alice.age プロパティが復元",
        rows and rows[0].get("n.age") == 30,
        str(rows),
    )

    resp = client2.query("MATCH (n:Item) RETURN n.idx")
    item_count2 = len(resp.get("rows", []))
    check("Item ノード100件が復元", item_count2 == 100, f"count={item_count2}")

    client2.close()
    stop_server(proc2, signal.SIGTERM)


def test_sigkill_roundtrip(binary: str, port: int, data_path: str):
    """強制終了（SIGKILL）後も、応答済みの書き込みが WAL から復元されること。"""
    section("SIGKILL 後の再起動")

    # SIGKILL 前に一度 SIGTERM で保存済みデータを用意しておく
    proc = start_server(binary, port, data_path)
    ready = wait_for_server("127.0.0.1", port)
    check("サーバー起動", ready)
    if not ready:
        proc.kill(); proc.wait()
        return

    client = MaharitClient("127.0.0.1", port)
    client.query("CREATE (n:Survivor {name: 'persistent'})")
    client.close()

    # 一旦 SIGTERM で保存
    stop_server(proc, signal.SIGTERM)

    # 再起動してから即 SIGKILL
    proc2 = start_server(binary, port, data_path)
    ready2 = wait_for_server("127.0.0.1", port)
    check("SIGKILL 前サーバー起動", ready2)
    if not ready2:
        proc2.kill(); proc2.wait()
        return

    # 書き込んでから SIGKILL。応答を受け取った書き込みは WAL（既定 --wal fsync）から復元される
    client2 = MaharitClient("127.0.0.1", port)
    client2.query("CREATE (n:Volatile {name: 'after_save', d: date('2024-02-29')})")
    client2.query("MATCH (n:Survivor) SET n.touched = true")
    client2.close()

    stop_server(proc2, signal.SIGKILL, wait_secs=3.0)

    # SIGKILL 後の再起動でクラッシュしないことを確認
    proc3 = start_server(binary, port, data_path)
    ready3 = wait_for_server("127.0.0.1", port)
    check("SIGKILL 後の再起動でクラッシュしない", ready3)

    if ready3:
        client3 = MaharitClient("127.0.0.1", port)
        resp = client3.query("MATCH (n:Survivor) RETURN n.name")
        check(
            "SIGTERM 保存データは SIGKILL 後も復元",
            len(resp.get("rows", [])) >= 1,
            str(resp),
        )
        resp = client3.query(
            "MATCH (n:Volatile) WHERE n.d = date('2024-02-29') RETURN n.name AS name"
        )
        check(
            "SIGKILL 直前の書き込みが WAL から復元（date 型も保持）",
            resp.get("rows") == [{"name": "after_save"}],
            str(resp),
        )
        resp = client3.query("MATCH (n:Survivor) RETURN n.touched AS t")
        check(
            "SIGKILL 直前の SET が WAL から復元",
            resp.get("rows") == [{"t": True}],
            str(resp),
        )
        client3.close()
        stop_server(proc3, signal.SIGTERM)
    else:
        proc3.kill(); proc3.wait()


# ── エントリポイント ──────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="MaharitDB 永続化ラウンドトリップテスト")
    parser.add_argument("--binary", default=None, help="maharit バイナリのパス")
    parser.add_argument("--port", type=int, default=7699, help="テスト用ポート (default: 7699)")
    parser.add_argument("--data", default=None, help="テスト用 DB ファイルパス（省略時は一時ファイル）")
    args = parser.parse_args()

    binary = args.binary or find_binary()
    print(f"バイナリ: {binary}")

    # データファイルは一時ディレクトリに作成（テスト後に削除）
    use_tmpdir = args.data is None
    if use_tmpdir:
        tmpdir = tempfile.mkdtemp(prefix="maharit_persist_test_")
        data_path = os.path.join(tmpdir, "test.db")
    else:
        tmpdir = None
        data_path = args.data

    print(f"データパス: {data_path}")
    print(f"ポート: {args.port}\n")

    try:
        test_sigterm_roundtrip(binary, args.port, data_path)

        # SIGKILL テスト用にデータファイルと WAL をリセット
        for path in (data_path, data_path + ".wal"):
            if os.path.isfile(path):
                os.remove(path)

        test_sigkill_roundtrip(binary, args.port, data_path)
    finally:
        if use_tmpdir and tmpdir and os.path.isdir(tmpdir):
            shutil.rmtree(tmpdir, ignore_errors=True)

    sys.exit(summarize())


if __name__ == "__main__":
    main()
