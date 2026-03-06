"""Asynchronous MaharitDB client (asyncio)."""

import asyncio
import json
import struct
from contextlib import asynccontextmanager
from typing import Any, AsyncGenerator, AsyncIterator, Dict, List, Optional

from .exceptions import ConnectionError, MaharitError, QueryError, TransactionError


_LENGTH_PREFIX_FORMAT = ">I"
_LENGTH_PREFIX_SIZE = struct.calcsize(_LENGTH_PREFIX_FORMAT)


async def _async_send(writer: asyncio.StreamWriter, payload: dict) -> None:
    data = json.dumps(payload).encode("utf-8")
    prefix = struct.pack(_LENGTH_PREFIX_FORMAT, len(data))
    writer.write(prefix + data)
    await writer.drain()


async def _async_recv(reader: asyncio.StreamReader) -> dict:
    prefix = await reader.readexactly(_LENGTH_PREFIX_SIZE)
    (length,) = struct.unpack(_LENGTH_PREFIX_FORMAT, prefix)
    data = await reader.readexactly(length)
    return json.loads(data.decode("utf-8"))


class AsyncQueryResult:
    """Result of an async query execution."""

    def __init__(self, rows: List[Dict[str, str]]) -> None:
        self._rows = rows

    def __iter__(self):
        return iter(self._rows)

    def __len__(self) -> int:
        return len(self._rows)

    def __getitem__(self, index: int) -> Dict[str, str]:
        return self._rows[index]

    def to_dataframe(self):
        """Convert the result to a pandas DataFrame."""
        try:
            import pandas as pd
        except ImportError as e:
            raise ImportError(
                "pandas is required for to_dataframe(). "
                "Install it with: pip install pandas"
            ) from e
        return pd.DataFrame(self._rows)

    def __repr__(self) -> str:
        return f"AsyncQueryResult({len(self._rows)} rows)"


class AsyncTransaction:
    """Represents an active asynchronous database transaction."""

    def __init__(self, client: "AsyncClient", tx_id: int) -> None:
        self._client = client
        self._tx_id = tx_id
        self._closed = False

    @property
    def tx_id(self) -> int:
        return self._tx_id

    async def execute(self, query: str) -> AsyncQueryResult:
        """Execute a query within this transaction."""
        if self._closed:
            raise TransactionError("Transaction is already closed")
        return await self._client._execute_query(query, tx_id=self._tx_id)

    async def commit(self) -> None:
        """Commit the transaction."""
        if self._closed:
            raise TransactionError("Transaction is already closed")
        await self._client._send({"type": "commit", "txId": self._tx_id})
        response = await self._client._recv()
        self._closed = True
        if response.get("type") == "error":
            raise TransactionError(response.get("message", "Commit failed"))
        if response.get("type") != "committed":
            raise TransactionError(f"Unexpected response: {response}")

    async def rollback(self) -> None:
        """Rollback the transaction."""
        if self._closed:
            return
        try:
            await self._client._send({"type": "rollback", "txId": self._tx_id})
            await self._client._recv()
        except Exception:
            pass
        finally:
            self._closed = True

    async def __aenter__(self) -> "AsyncTransaction":
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        if exc_type is None:
            await self.commit()
        else:
            await self.rollback()

    def __repr__(self) -> str:
        return f"AsyncTransaction(id={self._tx_id}, closed={self._closed})"


class AsyncClient:
    """Asynchronous MaharitDB client.

    Example::

        async with AsyncClient.connect("localhost:7687") as client:
            await client.execute("CREATE (n:Person {name: 'Alice'})")
            result = await client.query("MATCH (n:Person) RETURN n.name")
            async for row in client.stream("MATCH (n) RETURN n"):
                print(row)
    """

    def __init__(self, host: str, port: int, timeout: float = 30.0) -> None:
        self._host = host
        self._port = port
        self._timeout = timeout
        self._reader: Optional[asyncio.StreamReader] = None
        self._writer: Optional[asyncio.StreamWriter] = None

    @classmethod
    async def connect(cls, address: str, timeout: float = 30.0) -> "AsyncClient":
        """Connect to a MaharitDB server asynchronously.

        Args:
            address: Server address in the form ``host:port``.
            timeout: Connection timeout in seconds.

        Returns:
            A connected :class:`AsyncClient` instance.
        """
        host, _, port_str = address.rpartition(":")
        if not host:
            host = "localhost"
        port = int(port_str) if port_str else 7687
        client = cls(host, port, timeout)
        await client._connect()
        return client

    async def _connect(self) -> None:
        try:
            reader, writer = await asyncio.wait_for(
                asyncio.open_connection(self._host, self._port),
                timeout=self._timeout,
            )
        except (OSError, asyncio.TimeoutError) as e:
            raise ConnectionError(
                f"Cannot connect to {self._host}:{self._port}: {e}"
            ) from e
        self._reader = reader
        self._writer = writer

    async def _send(self, payload: dict) -> None:
        if self._writer is None:
            raise ConnectionError("Not connected")
        try:
            await _async_send(self._writer, payload)
        except OSError as e:
            raise ConnectionError(f"Send failed: {e}") from e

    async def _recv(self) -> dict:
        if self._reader is None:
            raise ConnectionError("Not connected")
        try:
            return await _async_recv(self._reader)
        except (OSError, asyncio.IncompleteReadError) as e:
            raise ConnectionError(f"Receive failed: {e}") from e

    async def ping(self) -> bool:
        """Ping the server."""
        await self._send({"type": "ping"})
        response = await self._recv()
        return response.get("type") == "pong"

    async def _execute_query(
        self, query: str, tx_id: Optional[int] = None
    ) -> AsyncQueryResult:
        payload: dict = {"type": "query", "query": query}
        if tx_id is not None:
            payload["txId"] = tx_id
        await self._send(payload)
        response = await self._recv()
        if response.get("type") == "error":
            raise QueryError(response.get("message", "Query failed"))
        if response.get("type") != "result":
            raise QueryError(f"Unexpected response type: {response.get('type')}")
        return AsyncQueryResult(response.get("rows", []))

    async def execute(self, query: str) -> AsyncQueryResult:
        """Execute a Cypher query."""
        return await self._execute_query(query)

    async def query(self, query: str) -> AsyncQueryResult:
        """Alias for :meth:`execute`."""
        return await self.execute(query)

    async def stream(
        self, query: str, chunk_size: int = 100
    ) -> AsyncIterator[Dict[str, str]]:
        """Stream query results asynchronously.

        Example::

            async for row in client.stream("MATCH (n) RETURN n"):
                print(row)
        """
        await self._send({"type": "streamQuery", "query": query, "chunkSize": chunk_size})

        start = await self._recv()
        if start.get("type") == "error":
            raise QueryError(start.get("message", "Stream failed"))
        if start.get("type") != "streamStart":
            raise QueryError(f"Expected streamStart, got: {start.get('type')}")

        while True:
            msg = await self._recv()
            msg_type = msg.get("type")
            if msg_type == "streamChunk":
                for row in msg.get("rows", []):
                    yield row
            elif msg_type == "streamEnd":
                break
            elif msg_type == "error":
                raise QueryError(msg.get("message", "Stream error"))
            else:
                raise QueryError(f"Unexpected message type: {msg_type}")

    async def begin_transaction(self, read_only: bool = False) -> AsyncTransaction:
        """Begin a new transaction."""
        await self._send({"type": "begin", "readOnly": read_only})
        response = await self._recv()
        if response.get("type") == "error":
            raise TransactionError(response.get("message", "Begin transaction failed"))
        if response.get("type") != "transactionBegun":
            raise TransactionError(f"Unexpected response: {response}")
        tx_id = response["txId"]
        return AsyncTransaction(self, tx_id)

    @asynccontextmanager
    async def transaction(
        self, read_only: bool = False
    ) -> AsyncGenerator["AsyncTransaction", None]:
        """Async context manager for transactions."""
        tx = await self.begin_transaction(read_only=read_only)
        try:
            yield tx
            if not tx._closed:
                await tx.commit()
        except Exception:
            if not tx._closed:
                await tx.rollback()
            raise

    async def stats(self) -> Dict[str, Any]:
        """Get server statistics."""
        await self._send({"type": "stats"})
        response = await self._recv()
        if response.get("type") != "stats":
            raise MaharitError(f"Unexpected response: {response}")
        return {
            "connections": response.get("connections", 0),
            "total_queries": response.get("total_queries", 0),
            "nodes": response.get("nodes", 0),
            "edges": response.get("edges", 0),
        }

    async def close(self) -> None:
        """Close the connection."""
        if self._writer is not None:
            try:
                await self._send({"type": "disconnect"})
                await self._recv()
            except Exception:
                pass
            finally:
                try:
                    self._writer.close()
                    await self._writer.wait_closed()
                except Exception:
                    pass
                self._reader = None
                self._writer = None

    async def __aenter__(self) -> "AsyncClient":
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        await self.close()

    def __repr__(self) -> str:
        connected = self._writer is not None
        return f"AsyncClient({self._host}:{self._port}, connected={connected})"
