"""Tests for the asynchronous AsyncClient."""

import asyncio
import json
import struct

import pytest

from maharit.async_client import AsyncClient, AsyncQueryResult, AsyncTransaction
from maharit.exceptions import ConnectionError, QueryError, TransactionError


# ---------------------------------------------------------------------------
# Helpers — in-process fake server using asyncio streams
# ---------------------------------------------------------------------------

async def _start_fake_server(responses):
    """Start an asyncio server that sends `responses` in order (one per request)."""
    async def handler(reader, writer):
        for resp in responses:
            # consume request
            try:
                prefix = await reader.readexactly(4)
                (length,) = struct.unpack(">I", prefix)
                await reader.readexactly(length)
            except asyncio.IncompleteReadError:
                break
            # send response
            data = json.dumps(resp).encode()
            writer.write(struct.pack(">I", len(data)) + data)
            await writer.drain()
        writer.close()

    server = await asyncio.start_server(handler, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    return server, port


# ---------------------------------------------------------------------------
# AsyncQueryResult
# ---------------------------------------------------------------------------

class TestAsyncQueryResult:
    def test_iteration(self):
        rows = [{"a": 1}, {"a": 2}]
        result = AsyncQueryResult(rows)
        assert list(result) == rows

    def test_len(self):
        assert len(AsyncQueryResult([])) == 0
        assert len(AsyncQueryResult([{}, {}])) == 2

    def test_getitem(self):
        result = AsyncQueryResult([{"x": 10}, {"x": 20}])
        assert result[0] == {"x": 10}

    def test_repr(self):
        assert "3 rows" in repr(AsyncQueryResult([{}, {}, {}]))


# ---------------------------------------------------------------------------
# AsyncClient via fake asyncio server
# ---------------------------------------------------------------------------

class TestAsyncClient:
    @pytest.mark.asyncio
    async def test_ping(self):
        server, port = await _start_fake_server([{"type": "pong"}])
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                result = await client.ping()
            assert result is True

    @pytest.mark.asyncio
    async def test_execute_success(self):
        rows = [{"n.name": "Alice"}, {"n.name": "Bob"}]
        server, port = await _start_fake_server([{"type": "result", "rows": rows}])
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                result = await client.execute("MATCH (n) RETURN n.name")
            assert isinstance(result, AsyncQueryResult)
            assert len(result) == 2
            assert result[0]["n.name"] == "Alice"

    @pytest.mark.asyncio
    async def test_execute_error(self):
        server, port = await _start_fake_server([
            {"type": "error", "message": "bad query"}
        ])
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            with pytest.raises(QueryError, match="bad query"):
                async with client:
                    await client.execute("BAD")

    @pytest.mark.asyncio
    async def test_query_alias(self):
        rows = [{"x": 99}]
        server, port = await _start_fake_server([{"type": "result", "rows": rows}])
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                result = await client.query("MATCH (n) RETURN n")
            assert list(result) == rows

    @pytest.mark.asyncio
    async def test_stream(self):
        async def handler(reader, writer):
            # consume streamQuery request
            prefix = await reader.readexactly(4)
            (length,) = struct.unpack(">I", prefix)
            await reader.readexactly(length)
            # send stream responses
            for resp in [
                {"type": "streamStart"},
                {"type": "streamChunk", "rows": [{"a": 1}, {"a": 2}]},
                {"type": "streamChunk", "rows": [{"a": 3}]},
                {"type": "streamEnd"},
                # disconnect message
                {"type": "disconnected"},
            ]:
                data = json.dumps(resp).encode()
                writer.write(struct.pack(">I", len(data)) + data)
                await writer.drain()
            writer.close()

        server = await asyncio.start_server(handler, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            rows = []
            async for row in client.stream("MATCH (n) RETURN n"):
                rows.append(row)
        assert rows == [{"a": 1}, {"a": 2}, {"a": 3}]

    @pytest.mark.asyncio
    async def test_stats(self):
        stat_resp = {
            "type": "stats",
            "connections": 5,
            "total_queries": 200,
            "nodes": 100,
            "edges": 40,
        }
        server, port = await _start_fake_server([stat_resp, {"type": "disconnected"}])
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                stats = await client.stats()
            assert stats["connections"] == 5
            assert stats["nodes"] == 100

    @pytest.mark.asyncio
    async def test_transaction_commit(self):
        async def handler(reader, writer):
            async def recv():
                prefix = await reader.readexactly(4)
                (n,) = struct.unpack(">I", prefix)
                return json.loads(await reader.readexactly(n))

            async def send(msg):
                data = json.dumps(msg).encode()
                writer.write(struct.pack(">I", len(data)) + data)
                await writer.drain()

            await recv()  # begin
            await send({"type": "transactionBegun", "txId": 10})
            await recv()  # commit
            await send({"type": "committed"})
            await recv()  # disconnect
            writer.close()

        server = await asyncio.start_server(handler, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                tx = await client.begin_transaction()
                assert tx.tx_id == 10
                await tx.commit()
                assert tx._closed

    @pytest.mark.asyncio
    async def test_transaction_context_manager_commits(self):
        async def handler(reader, writer):
            async def recv():
                prefix = await reader.readexactly(4)
                (n,) = struct.unpack(">I", prefix)
                return json.loads(await reader.readexactly(n))

            async def send(msg):
                data = json.dumps(msg).encode()
                writer.write(struct.pack(">I", len(data)) + data)
                await writer.drain()

            await recv()  # begin
            await send({"type": "transactionBegun", "txId": 11})
            await recv()  # query
            await send({"type": "result", "rows": []})
            await recv()  # commit
            await send({"type": "committed"})
            await recv()  # disconnect
            writer.close()

        server = await asyncio.start_server(handler, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                async with client.transaction() as tx:
                    await tx.execute("CREATE (n:Test)")

    @pytest.mark.asyncio
    async def test_execute_on_closed_tx_raises(self):
        async def handler(reader, writer):
            async def recv():
                prefix = await reader.readexactly(4)
                (n,) = struct.unpack(">I", prefix)
                return json.loads(await reader.readexactly(n))

            async def send(msg):
                data = json.dumps(msg).encode()
                writer.write(struct.pack(">I", len(data)) + data)
                await writer.drain()

            await recv()  # begin
            await send({"type": "transactionBegun", "txId": 99})
            await recv()  # rollback
            await send({"type": "rolledBack"})
            await recv()  # disconnect
            writer.close()

        server = await asyncio.start_server(handler, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        async with server:
            client = await AsyncClient.connect(f"127.0.0.1:{port}")
            async with client:
                tx = await client.begin_transaction()
                await tx.rollback()
                with pytest.raises(TransactionError):
                    await tx.execute("MATCH (n) RETURN n")
