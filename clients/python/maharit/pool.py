"""Connection pool for MaharitDB clients."""

import asyncio
import queue
import threading
from contextlib import asynccontextmanager, contextmanager
from typing import AsyncGenerator, Generator, Optional

from .async_client import AsyncClient
from .client import Client
from .exceptions import ConnectionError


class ConnectionPool:
    """Thread-safe synchronous connection pool.

    Maintains a pool of reusable :class:`Client` connections to avoid the
    overhead of establishing a new TCP connection for every query.

    Example::

        pool = ConnectionPool("localhost:7687", max_size=5)
        with pool.connection() as client:
            result = client.query("MATCH (n) RETURN n")

        # or as a context manager (closes all connections on exit)
        with ConnectionPool("localhost:7687") as pool:
            with pool.connection() as client:
                client.execute("CREATE (n:Person {name: 'Alice'})")
    """

    def __init__(
        self,
        address: str,
        max_size: int = 10,
        timeout: float = 30.0,
    ) -> None:
        self._address = address
        self._max_size = max_size
        self._timeout = timeout
        # idle connections
        self._idle: queue.Queue[Client] = queue.Queue(maxsize=max_size)
        self._lock = threading.Lock()
        self._total = 0  # total open connections (idle + checked-out)

    def _new_connection(self) -> Client:
        return Client.connect(self._address, timeout=self._timeout)

    def _is_alive(self, client: Client) -> bool:
        try:
            return client.ping()
        except Exception:
            return False

    def acquire(self) -> Client:
        """Acquire a connection from the pool.

        If a healthy idle connection is available it is returned immediately.
        Otherwise a new connection is created up to *max_size*.  If the pool
        is full the call blocks until one becomes available or *timeout*
        seconds elapse.

        Returns:
            A :class:`Client` ready to use.

        Raises:
            :class:`~maharit.exceptions.ConnectionError`: If no connection
                could be obtained within the timeout.
        """
        # Drain idle queue until we find a live connection or exhaust it
        while True:
            try:
                client = self._idle.get_nowait()
                if self._is_alive(client):
                    return client
                # Dead connection – discard and account for the loss
                try:
                    client.close()
                except Exception:
                    pass
                with self._lock:
                    self._total -= 1
            except queue.Empty:
                break

        # Create a new connection if capacity allows
        with self._lock:
            if self._total < self._max_size:
                client = self._new_connection()
                self._total += 1
                return client

        # Pool is full – wait for a connection to be released
        try:
            client = self._idle.get(timeout=self._timeout)
        except queue.Empty:
            raise ConnectionError(
                f"Connection pool exhausted (max_size={self._max_size}). "
                "No connection became available within the timeout."
            )
        if not self._is_alive(client):
            try:
                client.close()
            except Exception:
                pass
            with self._lock:
                self._total -= 1
            return self.acquire()
        return client

    def release(self, client: Client) -> None:
        """Return a connection to the pool.

        If the pool is already full the connection is closed.
        """
        try:
            self._idle.put_nowait(client)
        except queue.Full:
            try:
                client.close()
            except Exception:
                pass
            with self._lock:
                self._total -= 1

    @contextmanager
    def connection(self) -> Generator[Client, None, None]:
        """Context manager that acquires and releases a connection automatically.

        On any exception the connection is closed rather than returned to the
        pool, because its state may be undefined.

        Example::

            with pool.connection() as client:
                client.execute("CREATE (n:Test)")
        """
        client = self.acquire()
        try:
            yield client
        except Exception:
            # Don't return a potentially dirty connection to the pool
            try:
                client.close()
            except Exception:
                pass
            with self._lock:
                self._total -= 1
            raise
        else:
            self.release(client)

    def close_all(self) -> None:
        """Close all idle connections in the pool."""
        while True:
            try:
                client = self._idle.get_nowait()
                try:
                    client.close()
                except Exception:
                    pass
                with self._lock:
                    self._total -= 1
            except queue.Empty:
                break

    def __enter__(self) -> "ConnectionPool":
        return self

    def __exit__(self, *args: object) -> None:
        self.close_all()

    def __repr__(self) -> str:
        return (
            f"ConnectionPool({self._address!r}, "
            f"max_size={self._max_size}, total={self._total})"
        )


class AsyncConnectionPool:
    """Async connection pool backed by :class:`AsyncClient`.

    Example::

        pool = AsyncConnectionPool("localhost:7687", max_size=5)
        async with pool.connection() as client:
            result = await client.query("MATCH (n) RETURN n")

        async with AsyncConnectionPool("localhost:7687") as pool:
            async with pool.connection() as client:
                await client.execute("CREATE (n:Person {name: 'Bob'})")
    """

    def __init__(
        self,
        address: str,
        max_size: int = 10,
        timeout: float = 30.0,
    ) -> None:
        self._address = address
        self._max_size = max_size
        self._timeout = timeout
        self._idle: asyncio.Queue[AsyncClient] = asyncio.Queue(maxsize=max_size)
        self._lock = asyncio.Lock()
        self._total = 0

    async def _new_connection(self) -> AsyncClient:
        return await AsyncClient.connect(self._address, timeout=self._timeout)

    async def _is_alive(self, client: AsyncClient) -> bool:
        try:
            return await client.ping()
        except Exception:
            return False

    async def acquire(self) -> AsyncClient:
        """Acquire a connection from the async pool.

        Returns:
            A connected :class:`AsyncClient`.

        Raises:
            :class:`~maharit.exceptions.ConnectionError`: If no connection
                became available within the timeout.
        """
        while True:
            try:
                client = self._idle.get_nowait()
                if await self._is_alive(client):
                    return client
                try:
                    await client.close()
                except Exception:
                    pass
                async with self._lock:
                    self._total -= 1
            except asyncio.QueueEmpty:
                break

        async with self._lock:
            if self._total < self._max_size:
                client = await self._new_connection()
                self._total += 1
                return client

        try:
            client = await asyncio.wait_for(self._idle.get(), timeout=self._timeout)
        except asyncio.TimeoutError:
            raise ConnectionError(
                f"Async connection pool exhausted (max_size={self._max_size}). "
                "No connection became available within the timeout."
            )
        if not await self._is_alive(client):
            try:
                await client.close()
            except Exception:
                pass
            async with self._lock:
                self._total -= 1
            return await self.acquire()
        return client

    async def release(self, client: AsyncClient) -> None:
        """Return a connection to the async pool."""
        try:
            self._idle.put_nowait(client)
        except asyncio.QueueFull:
            try:
                await client.close()
            except Exception:
                pass
            async with self._lock:
                self._total -= 1

    @asynccontextmanager
    async def connection(self) -> AsyncGenerator[AsyncClient, None]:
        """Async context manager that acquires and releases a connection.

        Example::

            async with pool.connection() as client:
                await client.execute("CREATE (n:Test)")
        """
        client = await self.acquire()
        try:
            yield client
        except Exception:
            try:
                await client.close()
            except Exception:
                pass
            async with self._lock:
                self._total -= 1
            raise
        else:
            await self.release(client)

    async def close_all(self) -> None:
        """Close all idle connections in the async pool."""
        while True:
            try:
                client = self._idle.get_nowait()
                try:
                    await client.close()
                except Exception:
                    pass
                async with self._lock:
                    self._total -= 1
            except asyncio.QueueEmpty:
                break

    async def __aenter__(self) -> "AsyncConnectionPool":
        return self

    async def __aexit__(self, *args: object) -> None:
        await self.close_all()

    def __repr__(self) -> str:
        return (
            f"AsyncConnectionPool({self._address!r}, "
            f"max_size={self._max_size}, total={self._total})"
        )
