"""Tests for the synchronous Client."""

import json
import socket
import struct
import threading
from unittest.mock import MagicMock, patch

import pytest

from maharit.client import Client, QueryResult, Transaction
from maharit.exceptions import ConnectionError, QueryError, TransactionError
from maharit.protocol import encode_message, recv_message, send_message


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_server(responses):
    """Create a socket pair. Server side sends each response after reading one request."""
    server_sock, client_sock = socket.socketpair()

    def _serve():
        for resp in responses:
            try:
                recv_message(server_sock)  # consume the request
                send_message(server_sock, resp)
            except Exception:
                break
        server_sock.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()
    return client_sock, t


def _client_with_sock(sock):
    """Return a Client wired to an already-connected socket."""
    client = Client.__new__(Client)
    client._host = "localhost"
    client._port = 7687
    client._timeout = 30.0
    client._max_retries = 0
    client._retry_delay = 0.0
    client._sock = sock
    return client


# ---------------------------------------------------------------------------
# QueryResult
# ---------------------------------------------------------------------------

class TestQueryResult:
    def test_iteration(self):
        rows = [{"a": 1}, {"a": 2}]
        result = QueryResult(rows)
        assert list(result) == rows

    def test_len(self):
        assert len(QueryResult([{"x": 1}])) == 1
        assert len(QueryResult([])) == 0

    def test_getitem(self):
        rows = [{"a": 1}, {"b": 2}]
        result = QueryResult(rows)
        assert result[0] == {"a": 1}
        assert result[1] == {"b": 2}

    def test_repr(self):
        assert "2 rows" in repr(QueryResult([{}, {}]))

    def test_to_dataframe_no_pandas(self):
        result = QueryResult([{"a": 1}])
        with patch.dict("sys.modules", {"pandas": None}):
            with pytest.raises(ImportError, match="pandas"):
                result.to_dataframe()


# ---------------------------------------------------------------------------
# Client.connect address parsing
# ---------------------------------------------------------------------------

class TestAddressParsing:
    def _connect_args(self, address):
        calls = []
        with patch.object(Client, "_connect", lambda self: calls.append((self._host, self._port))):
            Client.connect(address)
        return calls[0]

    def test_host_port(self):
        host, port = self._connect_args("localhost:7687")
        assert host == "localhost"
        assert port == 7687

    def test_port_only(self):
        # When no host supplied the rpartition leaves host="" → fallback to localhost
        host, port = self._connect_args(":9000")
        assert host == "localhost"
        assert port == 9000

    def test_ip_port(self):
        host, port = self._connect_args("127.0.0.1:1234")
        assert host == "127.0.0.1"
        assert port == 1234


# ---------------------------------------------------------------------------
# Client methods via socket pair
# ---------------------------------------------------------------------------

class TestClientMethods:
    def test_ping_true(self):
        sock, _ = _make_server([{"type": "pong"}])
        client = _client_with_sock(sock)
        assert client.ping() is True

    def test_ping_false(self):
        sock, _ = _make_server([{"type": "error", "message": "nope"}])
        client = _client_with_sock(sock)
        assert client.ping() is False

    def test_execute_success(self):
        rows = [{"n.name": "Alice"}, {"n.name": "Bob"}]
        sock, _ = _make_server([{"type": "result", "rows": rows}])
        client = _client_with_sock(sock)
        result = client.execute("MATCH (n) RETURN n.name")
        assert isinstance(result, QueryResult)
        assert len(result) == 2
        assert result[0]["n.name"] == "Alice"

    def test_execute_error(self):
        sock, _ = _make_server([{"type": "error", "message": "syntax error"}])
        client = _client_with_sock(sock)
        with pytest.raises(QueryError, match="syntax error"):
            client.execute("BAD QUERY")

    def test_query_alias(self):
        rows = [{"x": 1}]
        sock, _ = _make_server([{"type": "result", "rows": rows}])
        client = _client_with_sock(sock)
        result = client.query("MATCH (n) RETURN n")
        assert list(result) == rows

    def test_stream(self):
        responses = [
            {"type": "streamStart"},
            {"type": "streamChunk", "rows": [{"a": 1}, {"a": 2}]},
            {"type": "streamChunk", "rows": [{"a": 3}]},
            {"type": "streamEnd"},
        ]
        server_sock, client_sock = socket.socketpair()

        def _serve():
            _recv = lambda: recv_message(server_sock)
            _send = lambda m: send_message(server_sock, m)
            _recv()  # streamQuery request
            for resp in responses:
                _send(resp)
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        rows = list(client.stream("MATCH (n) RETURN n"))
        assert rows == [{"a": 1}, {"a": 2}, {"a": 3}]

    def test_stats(self):
        stat_resp = {
            "type": "stats",
            "connections": 3,
            "total_queries": 100,
            "nodes": 50,
            "edges": 20,
        }
        sock, _ = _make_server([stat_resp])
        client = _client_with_sock(sock)
        stats = client.stats()
        assert stats["connections"] == 3
        assert stats["nodes"] == 50

    def test_context_manager_calls_close(self):
        sock, _ = _make_server([
            {"type": "disconnected"},
        ])
        client = _client_with_sock(sock)
        with client:
            pass
        assert client._sock is None


# ---------------------------------------------------------------------------
# Transaction
# ---------------------------------------------------------------------------

class TestTransaction:
    def test_commit(self):
        server_sock, client_sock = socket.socketpair()

        def _serve():
            recv_message(server_sock)  # begin
            send_message(server_sock, {"type": "transactionBegun", "txId": 1})
            recv_message(server_sock)  # commit
            send_message(server_sock, {"type": "committed"})
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        tx = client.begin_transaction()
        assert tx.tx_id == 1
        tx.commit()
        assert tx._closed

    def test_rollback(self):
        server_sock, client_sock = socket.socketpair()

        def _serve():
            recv_message(server_sock)  # begin
            send_message(server_sock, {"type": "transactionBegun", "txId": 7})
            recv_message(server_sock)  # rollback
            send_message(server_sock, {"type": "rolledBack"})
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        tx = client.begin_transaction()
        tx.rollback()
        assert tx._closed

    def test_context_manager_commits_on_success(self):
        server_sock, client_sock = socket.socketpair()

        def _serve():
            recv_message(server_sock)  # begin
            send_message(server_sock, {"type": "transactionBegun", "txId": 2})
            recv_message(server_sock)  # query
            send_message(server_sock, {"type": "result", "rows": []})
            recv_message(server_sock)  # commit
            send_message(server_sock, {"type": "committed"})
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        with client.transaction() as tx:
            tx.execute("CREATE (n:Test)")

    def test_context_manager_rolls_back_on_exception(self):
        server_sock, client_sock = socket.socketpair()

        def _serve():
            recv_message(server_sock)  # begin
            send_message(server_sock, {"type": "transactionBegun", "txId": 3})
            recv_message(server_sock)  # rollback
            send_message(server_sock, {"type": "rolledBack"})
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        with pytest.raises(ValueError):
            with client.transaction() as tx:
                raise ValueError("intentional")

    def test_execute_on_closed_transaction_raises(self):
        server_sock, client_sock = socket.socketpair()

        def _serve():
            recv_message(server_sock)  # begin
            send_message(server_sock, {"type": "transactionBegun", "txId": 4})
            recv_message(server_sock)  # rollback
            send_message(server_sock, {"type": "rolledBack"})
            server_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()
        client = _client_with_sock(client_sock)
        tx = client.begin_transaction()
        tx.rollback()
        with pytest.raises(TransactionError):
            tx.execute("MATCH (n) RETURN n")
