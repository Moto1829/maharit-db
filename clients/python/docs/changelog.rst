Changelog
=========

0.1.0 (2024)
------------

Initial release.

- Synchronous ``Client`` with connection management
- Asynchronous ``AsyncClient`` (asyncio)
- ``ConnectionPool`` and ``AsyncConnectionPool``
- ``Node`` and ``Edge`` Python model classes
- pandas ``DataFrame`` integration (optional dependency)
- Auto-reconnect with configurable retry policy
- Streaming query results with ``stream()`` / ``async_stream()``
- Transaction support (``BEGIN`` / ``COMMIT`` / ``ROLLBACK``)
- Parameter binding via ``$param`` syntax
