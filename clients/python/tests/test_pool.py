"""Tests for ConnectionPool and AsyncConnectionPool."""

import asyncio
import socket
import threading
from unittest.mock import MagicMock, patch, AsyncMock

import pytest

from maharit.client import Client
from maharit.async_client import AsyncClient
from maharit.exceptions import ConnectionError
from maharit.pool import AsyncConnectionPool, ConnectionPool
from maharit.protocol import recv_message, send_message


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_pong_server(count: int):
    """Create a socket-pair server that responds to `count` ping messages with pong."""
    server_sock, client_sock = socket.socketpair()

    def _serve():
        for _ in range(count):
            try:
                recv_message(server_sock)
                send_message(server_sock, {"type": "pong"})
            except Exception:
                break
        server_sock.close()

    t = threading.Thread(target=_serve, daemon=True)
    t.start()
    return client_sock, t


def _make_client(sock) -> Client:
    """Build a Client wired to the given socket without a real TCP connection."""
    client = Client.__new__(Client)
    client._host = "localhost"
    client._port = 7687
    client._timeout = 30.0
    client._max_retries = 0
    client._retry_delay = 0.0
    client._sock = sock
    return client


# ---------------------------------------------------------------------------
# ConnectionPool
# ---------------------------------------------------------------------------

class TestConnectionPool:
    def test_acquire_creates_new_connection(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        with patch.object(pool, "_new_connection", return_value=mock_client):
            client = pool.acquire()
            assert client is mock_client
            assert pool._total == 1

    def test_release_returns_to_idle(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        with patch.object(pool, "_new_connection", return_value=mock_client):
            client = pool.acquire()
            pool.release(client)
            assert pool._idle.qsize() == 1

    def test_acquire_reuses_idle_connection(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        with patch.object(pool, "_new_connection", return_value=mock_client):
            c1 = pool.acquire()
            pool.release(c1)
            c2 = pool.acquire()
            assert c2 is mock_client

    def test_dead_idle_connection_discarded(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        dead_client = MagicMock(spec=Client)
        dead_client.ping.return_value = False
        live_client = MagicMock(spec=Client)
        live_client.ping.return_value = True

        pool._idle.put_nowait(dead_client)
        pool._total = 1  # account for the dead one

        new_clients = iter([live_client])
        with patch.object(pool, "_new_connection", side_effect=new_clients):
            client = pool.acquire()
            assert client is live_client
            dead_client.close.assert_called_once()

    def test_connection_context_manager_releases_on_success(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        with patch.object(pool, "_new_connection", return_value=mock_client):
            with pool.connection() as client:
                assert client is mock_client
            assert pool._idle.qsize() == 1

    def test_connection_context_manager_closes_on_exception(self):
        pool = ConnectionPool("localhost:7687", max_size=3)
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        with patch.object(pool, "_new_connection", return_value=mock_client):
            with pytest.raises(ValueError):
                with pool.connection() as client:
                    raise ValueError("boom")
            mock_client.close.assert_called_once()
            assert pool._idle.qsize() == 0

    def test_pool_as_context_manager_closes_all(self):
        mock_client = MagicMock(spec=Client)
        mock_client.ping.return_value = True

        pool = ConnectionPool("localhost:7687", max_size=3)
        with patch.object(pool, "_new_connection", return_value=mock_client):
            with pool as p:
                with p.connection():
                    pass  # acquires then releases back to idle

        mock_client.close.assert_called_once()

    def test_repr(self):
        pool = ConnectionPool("localhost:7687", max_size=5)
        r = repr(pool)
        assert "localhost:7687" in r
        assert "5" in r

    def test_max_size_respected(self):
        pool = ConnectionPool("localhost:7687", max_size=2)
        clients = [MagicMock(spec=Client) for _ in range(2)]
        for c in clients:
            c.ping.return_value = True

        it = iter(clients)
        with patch.object(pool, "_new_connection", side_effect=it):
            c1 = pool.acquire()
            c2 = pool.acquire()
            assert pool._total == 2

            # Third acquire should block; use a thread to release after short delay
            released = threading.Event()

            def _release_later():
                import time
                time.sleep(0.05)
                pool.release(c1)
                released.set()

            t = threading.Thread(target=_release_later, daemon=True)
            t.start()

            c3 = pool.acquire()
            assert released.is_set()
            assert c3 is c1


# ---------------------------------------------------------------------------
# Auto-reconnect (Client._send retries)
# ---------------------------------------------------------------------------

class TestAutoReconnect:
    def test_send_retries_on_os_error(self):
        """_send should attempt reconnect and retry on OSError."""
        pool_sock, client_sock = socket.socketpair()

        client = _make_client(client_sock)
        client._max_retries = 2
        client._retry_delay = 0.0

        call_count = {"n": 0}
        original_send = send_message

        def _flaky_send(sock, payload):
            call_count["n"] += 1
            if call_count["n"] < 2:
                raise OSError("simulated failure")
            original_send(sock, payload)

        recv_responses = [{"type": "pong"}]

        def _serve():
            recv_message(pool_sock)
            send_message(pool_sock, {"type": "pong"})
            pool_sock.close()

        t = threading.Thread(target=_serve, daemon=True)
        t.start()

        reconnected = {"done": False}

        def _mock_reconnect():
            reconnected["done"] = True

        client._reconnect = _mock_reconnect

        with patch("maharit.client.send_message", side_effect=_flaky_send):
            client._send({"type": "ping"})

        assert call_count["n"] == 2
        assert reconnected["done"]

    def test_send_raises_after_max_retries_exceeded(self):
        sock, _ = socket.socketpair()
        client = _make_client(sock)
        client._max_retries = 1
        client._retry_delay = 0.0
        client._reconnect = lambda: None

        with patch("maharit.client.send_message", side_effect=OSError("always fails")):
            with pytest.raises(ConnectionError, match="Send failed after"):
                client._send({"type": "ping"})

    def test_connect_stores_retry_params(self):
        with patch.object(Client, "_connect", lambda self: None):
            client = Client.connect("localhost:7687", max_retries=5, retry_delay=2.0)
        assert client._max_retries == 5
        assert client._retry_delay == 2.0


# ---------------------------------------------------------------------------
# AsyncConnectionPool
# ---------------------------------------------------------------------------

class TestAsyncConnectionPool:
    @pytest.mark.asyncio
    async def test_acquire_creates_new_connection(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        mock_client = AsyncMock(spec=AsyncClient)
        mock_client.ping = AsyncMock(return_value=True)

        with patch.object(pool, "_new_connection", AsyncMock(return_value=mock_client)):
            client = await pool.acquire()
            assert client is mock_client
            assert pool._total == 1

    @pytest.mark.asyncio
    async def test_release_returns_to_idle(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        mock_client = AsyncMock(spec=AsyncClient)
        mock_client.ping = AsyncMock(return_value=True)

        with patch.object(pool, "_new_connection", AsyncMock(return_value=mock_client)):
            client = await pool.acquire()
            await pool.release(client)
            assert pool._idle.qsize() == 1

    @pytest.mark.asyncio
    async def test_connection_context_manager(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        mock_client = AsyncMock(spec=AsyncClient)
        mock_client.ping = AsyncMock(return_value=True)

        with patch.object(pool, "_new_connection", AsyncMock(return_value=mock_client)):
            async with pool.connection() as client:
                assert client is mock_client
            assert pool._idle.qsize() == 1

    @pytest.mark.asyncio
    async def test_connection_context_manager_closes_on_exception(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        mock_client = AsyncMock(spec=AsyncClient)
        mock_client.ping = AsyncMock(return_value=True)

        with patch.object(pool, "_new_connection", AsyncMock(return_value=mock_client)):
            with pytest.raises(ValueError):
                async with pool.connection() as client:
                    raise ValueError("boom")
            mock_client.close.assert_called_once()

    @pytest.mark.asyncio
    async def test_close_all(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        mock_client = AsyncMock(spec=AsyncClient)
        mock_client.ping = AsyncMock(return_value=True)

        with patch.object(pool, "_new_connection", AsyncMock(return_value=mock_client)):
            async with pool as p:
                async with p.connection():
                    pass

        mock_client.close.assert_called_once()

    @pytest.mark.asyncio
    async def test_dead_idle_connection_discarded(self):
        pool = AsyncConnectionPool("localhost:7687", max_size=3)
        dead_client = AsyncMock(spec=AsyncClient)
        dead_client.ping = AsyncMock(return_value=False)
        live_client = AsyncMock(spec=AsyncClient)
        live_client.ping = AsyncMock(return_value=True)

        pool._idle.put_nowait(dead_client)
        pool._total = 1

        new_clients = [live_client]
        it = iter(new_clients)

        async def _new():
            return next(it)

        with patch.object(pool, "_new_connection", side_effect=_new):
            client = await pool.acquire()
            assert client is live_client
            dead_client.close.assert_called_once()
