Quick Start
===========

Installation
------------

Install from PyPI::

    pip install maharit

For pandas DataFrame support::

    pip install maharit[pandas]

Connecting to MaharitDB
------------------------

Synchronous client
~~~~~~~~~~~~~~~~~~

.. code-block:: python

   from maharit import Client

   # Context manager (recommended)
   with Client.connect("localhost:7687") as client:
       client.execute("CREATE (n:Person {name: 'Alice'})")

   # Manual lifecycle
   client = Client.connect("localhost:7687")
   client.execute("CREATE (n:Person {name: 'Bob'})")
   client.close()

Asynchronous client
~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   import asyncio
   from maharit import AsyncClient

   async def main():
       async with AsyncClient.connect("localhost:7687") as client:
           await client.execute("CREATE (n:Person {name: 'Charlie'})")
           result = await client.query("MATCH (n:Person) RETURN n.name")
           for row in result:
               print(row["n.name"])

   asyncio.run(main())

Running Queries
---------------

.. code-block:: python

   with Client.connect("localhost:7687") as client:
       # Execute (no result expected)
       client.execute("CREATE (n:Item {id: 1})")

       # Query with results
       result = client.query("MATCH (n:Item) RETURN n.id")
       for row in result:
           print(row["n.id"])

       # Streaming (memory efficient for large result sets)
       for row in client.stream("MATCH (n) RETURN n", chunk_size=50):
           process(row)

Transactions
------------

.. code-block:: python

   with Client.connect("localhost:7687") as client:
       with client.transaction() as tx:
           tx.execute("CREATE (a:Person {name: 'Alice'})")
           tx.execute("CREATE (b:Person {name: 'Bob'})")
           # Commits automatically on __exit__; rolls back on exception

pandas DataFrame
----------------

.. code-block:: python

   with Client.connect("localhost:7687") as client:
       df = client.query(
           "MATCH (n:Person) RETURN n.name AS name, n.age AS age"
       ).to_dataframe()
       print(df.head())

Connection Pool
---------------

.. code-block:: python

   from maharit import ConnectionPool

   pool = ConnectionPool("localhost:7687", max_size=10)
   with pool.connection() as client:
       result = client.query("MATCH (n) RETURN count(n) AS cnt")

Node and Edge Objects
---------------------

.. code-block:: python

   from maharit import Client, Node, Edge

   with Client.connect("localhost:7687") as client:
       result = client.query("MATCH (n:Person) RETURN n")
       for row in result:
           node = Node.from_dict(row["n"])
           print(node["name"], node.labels)
