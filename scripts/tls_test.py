#!/usr/bin/env python3
"""
MaharitDB TLS テスト

自己署名証明書を作り、`--tls-cert` / `--tls-key` 付きでサーバーを起動して、
Python クライアント（同期・非同期）が TLS で接続できること、平文や
信頼していない証明書では接続できないこと、設定の誤りで起動を拒否することを
検証する。リーダーとフォロワーの間のレプリケーションを TLS で行えることも
検証する。証明書の作成に openssl コマンドを使う。

使い方:
  python3 scripts/tls_test.py
  python3 scripts/tls_test.py --binary ./target/release/maharit --port 7697
"""

import argparse
import asyncio
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "clients", "python"))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import check, section, summarize  # noqa: E402
from maharit import AsyncClient, Client  # noqa: E402
from maharit import ConnectionError as MaharitConnectionError  # noqa: E402


def find_binary() -> str:
    for path in (
        os.path.join(ROOT, "target", "release", "maharit"),
        os.path.join(ROOT, "target", "debug", "maharit"),
    ):
        if os.path.isfile(path):
            return path
    raise FileNotFoundError("maharit バイナリが見つかりません。先に `cargo build` を実行してください。")


def make_cert(directory: str, name: str) -> tuple[str, str]:
    """localhost / 127.0.0.1 用の自己署名サーバー証明書（CA:FALSE）を作る。"""
    cert = os.path.join(directory, f"{name}.crt")
    key = os.path.join(directory, f"{name}.key")
    subprocess.run(
        [
            "openssl", "req", "-x509", "-newkey", "ec",
            "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
            "-keyout", key, "-out", cert, "-days", "1", "-subj", "/CN=localhost",
            "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext", "basicConstraints=critical,CA:FALSE",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return cert, key


def start_server(binary: str, port: int, data: str, extra: list[str]) -> subprocess.Popen:
    return subprocess.Popen(
        [
            binary, "server", "--port", str(port), "--data", data, "--wal", "off",
            "--metrics-bind", f"127.0.0.1:{port + 2000}", *extra,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def wait_tls_ready(port: int, ca_file: str, timeout: float = 10.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with Client.connect(f"127.0.0.1:{port}", timeout=1.0, max_retries=0, ca_file=ca_file) as c:
                if c.ping():
                    return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def stop(proc: subprocess.Popen) -> str:
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    return proc.stdout.read() if proc.stdout else ""


def test_tls_connections(binary: str, port: int, tmp: str) -> None:
    section("TLS で接続する")
    cert, key = make_cert(tmp, "server")
    other_cert, _ = make_cert(tmp, "other")
    proc = start_server(
        binary, port, os.path.join(tmp, "tls.db"), ["--tls-cert", cert, "--tls-key", key]
    )
    try:
        ready = wait_tls_ready(port, cert)
        check("TLS サーバー起動・ping", ready)
        if not ready:
            return
        addr = f"127.0.0.1:{port}"

        with Client.connect(addr, ca_file=cert) as c:
            c.execute("CREATE (:Secret {name: $name})", params={"name": "tls"})
            rows = list(c.query("MATCH (s:Secret) RETURN s.name AS name"))
            check("同期クライアント: 書き込みと読み出し", rows == [{"name": "tls"}], str(rows))

        async def async_roundtrip() -> list:
            client = await AsyncClient.connect(addr, ca_file=cert)
            try:
                return list(await client.query("MATCH (s:Secret) RETURN s.name AS name"))
            finally:
                await client.close()

        rows = asyncio.run(async_roundtrip())
        check("非同期クライアント: 読み出し", rows == [{"name": "tls"}], str(rows))

        # 平文のクライアントには応答しない
        answered = False
        try:
            plain = MaharitClient("127.0.0.1", port, timeout=3.0)
            answered = plain.ping()
            plain.close()
        except Exception:
            pass
        check("平文の接続には応答しない", not answered)

        # 信頼していない証明書・システムの CA ストアでは検証に失敗する
        for label, kwargs in (
            ("別の証明書を信頼したクライアントは接続できない", {"ca_file": other_cert}),
            ("システムの CA ストアだけでは自己署名証明書を信頼しない", {"ssl": True}),
        ):
            rejected = False
            try:
                Client.connect(addr, timeout=3.0, max_retries=0, **kwargs).close()
            except MaharitConnectionError:
                rejected = True
            check(label, rejected)

        check("TLS 接続後もサーバーが動作中", proc.poll() is None)
    finally:
        output = stop(proc)
    check("起動ログに TLS が出る", "(TLS)" in output, " / ".join(output.strip().splitlines()[:3]))


def test_misconfiguration(binary: str, port: int, tmp: str) -> None:
    section("設定の誤りでは起動しない")
    cert, key = make_cert(tmp, "misc")
    cases = [
        ("--tls-cert だけ指定", ["--tls-cert", cert]),
        ("存在しない鍵ファイル", ["--tls-cert", cert, "--tls-key", os.path.join(tmp, "missing.key")]),
        ("証明書と鍵を取り違える", ["--tls-cert", key, "--tls-key", cert]),
        ("--tls-min-version が不正", ["--tls-cert", cert, "--tls-key", key, "--tls-min-version", "1.1"]),
    ]
    for label, extra in cases:
        proc = start_server(binary, port, os.path.join(tmp, "misc.db"), extra)
        try:
            code = proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            code = None
        output = stop(proc)
        check(f"{label}: 異常終了する", code not in (None, 0), output.strip()[-200:])


def wait_rows(port: int, ca_file: str, query: str, expected: list, timeout: float = 10.0) -> list:
    """クエリ結果が expected になるまで待ち、最後の結果を返す。"""
    deadline = time.time() + timeout
    rows: list = []
    while time.time() < deadline:
        try:
            with Client.connect(f"127.0.0.1:{port}", timeout=2.0, max_retries=0, ca_file=ca_file) as c:
                rows = list(c.query(query))
                if rows == expected:
                    return rows
        except Exception:
            pass
        time.sleep(0.2)
    return rows


def admin_promote(binary: str, addr: str, extra: list[str]) -> tuple[int, str]:
    result = subprocess.run(
        [binary, "admin", "promote-to-leader", "--addr", addr, *extra],
        capture_output=True, text=True, timeout=30,
        env={k: v for k, v in os.environ.items() if k != "MAHARIT_ADMIN_PASSWORD"},
    )
    return result.returncode, (result.stdout + result.stderr).strip()


def test_replication_over_tls(binary: str, port: int, tmp: str) -> None:
    section("レプリケーションを TLS で行う")
    cert, key = make_cert(tmp, "repl")
    other_cert, _ = make_cert(tmp, "repl_other")
    repl_port = port + 1
    tls_flags = ["--tls-cert", cert, "--tls-key", key]
    secret = ["--replication-secret", "tls-e2e-secret"]
    leader = start_server(
        binary, port, os.path.join(tmp, "leader.db"),
        [*tls_flags, *secret, "--replication-role", "leader",
         "--replication-bind", f"127.0.0.1:{repl_port}", "--node-id", "leader"],
    )
    followers: list[subprocess.Popen] = []
    try:
        check("TLS リーダー起動", wait_tls_ready(port, cert))
        follower_specs = [
            # (名前, 追加フラグ, データが届くべきか)
            ("trusted", ["--leader-ca", cert], True),
            ("plaintext", [], False),
            ("untrusted", ["--leader-ca", other_cert], False),
            ("secured", ["--leader-ca", cert, "--require-auth", "--admin-password", "pw-e2e"], None),
        ]
        for i, (name, extra, _) in enumerate(follower_specs):
            followers.append(start_server(
                binary, port + 2 + i, os.path.join(tmp, f"{name}.db"),
                [*tls_flags, *secret, "--replication-role", "follower",
                 "--leader-addr", f"127.0.0.1:{repl_port}", "--node-id", name,
                 # 昇格したときにレプリケーションを待ち受けるアドレス
                 "--replication-bind", f"127.0.0.1:{port + 20 + i}", *extra],
            ))
        for i, (name, _, _) in enumerate(follower_specs):
            check(f"フォロワー起動（{name}）", wait_tls_ready(port + 2 + i, cert))
        time.sleep(1)

        with Client.connect(f"127.0.0.1:{port}", ca_file=cert) as c:
            c.execute("CREATE (:Replicated {name: 'over-tls'})")
        query = "MATCH (r:Replicated) RETURN r.name AS name"
        expected = [{"name": "over-tls"}]
        for i, (name, _, should_receive) in enumerate(follower_specs):
            if should_receive is None:
                continue
            if should_receive:
                rows = wait_rows(port + 2 + i, cert, query, expected)
                check("--leader-ca で検証したフォロワーにデータが届く", rows == expected, str(rows))
            else:
                rows = wait_rows(port + 2 + i, cert, query, expected, timeout=3.0)
                label = {
                    "plaintext": "--leader-ca なしのフォロワーには届かない",
                    "untrusted": "別の証明書を信頼したフォロワーには届かない",
                }[name]
                check(label, rows != expected, str(rows))

        section("フォロワーを昇格する（TLS・認証）")
        trusted = f"127.0.0.1:{port + 2}"
        with Client.connect(trusted, ca_file=cert) as c:
            rejected = False
            try:
                c.execute("CREATE (:Rogue)")
            except Exception as e:
                rejected = "read-only follower" in str(e)
            check("昇格前のフォロワーは書き込みを拒否する", rejected)
        rc, out = admin_promote(binary, trusted, ["--ca", cert])
        check("admin promote-to-leader --ca で昇格できる", rc == 0 and "promoted" in out, out)
        with Client.connect(trusted, ca_file=cert) as c:
            c.execute("CREATE (:AfterPromotion)")
            rows = list(c.query("MATCH (n:AfterPromotion) RETURN count(n) AS c"))
            check("昇格後は書き込める", rows == [{"c": 1}], str(rows))
        rc, out = admin_promote(binary, trusted, ["--ca", cert])
        check("2 回目の昇格は拒否される", rc != 0, out)

        secured = f"127.0.0.1:{port + 5}"
        rc, out = admin_promote(binary, secured, ["--ca", cert])
        check("--require-auth のフォロワーはログインなしでは昇格できない", rc != 0, out)
        rc, out = admin_promote(binary, secured, ["--ca", cert, "--password", "wrong"])
        check("誤ったパスワードでは昇格できない", rc != 0, out)
        rc, out = admin_promote(binary, secured, ["--ca", cert, "--password", "pw-e2e"])
        check("管理者でログインすると昇格できる", rc == 0, out)
    finally:
        for proc in followers:
            stop(proc)
        output = stop(leader)
    check("リーダーの起動ログに TLS のレプリケーションが出る",
          "Replication connections use TLS" in output)


def main() -> None:
    parser = argparse.ArgumentParser(description="MaharitDB TLS テスト")
    parser.add_argument("--binary", default=None)
    parser.add_argument("--port", type=int, default=7697)
    args = parser.parse_args()

    if shutil.which("openssl") is None:
        print("openssl コマンドが見つからないため TLS テストを実行できません")
        sys.exit(1)
    binary = args.binary or find_binary()
    print(f"バイナリ: {binary}\nポート: {args.port}\n")

    tmp = tempfile.mkdtemp(prefix="maharit_tls_test_")
    try:
        test_tls_connections(binary, args.port, tmp)
        test_misconfiguration(binary, args.port, tmp)
        test_replication_over_tls(binary, args.port, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    sys.exit(summarize())


if __name__ == "__main__":
    main()
