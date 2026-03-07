maharit — Python client for MaharitDB
======================================

**maharit** is the official Python client for `MaharitDB <https://github.com/Moto1829/maharit-db>`_,
a high-performance graph database with Cypher query language support.

.. toctree::
   :maxdepth: 2
   :caption: Contents

   quickstart
   api
   changelog

Installation
------------

.. code-block:: bash

   pip install maharit

   # with pandas support
   pip install maharit[pandas]

Quick example
-------------

.. code-block:: python

   from maharit import Client

   with Client.connect("localhost:7687") as client:
       client.execute("CREATE (n:Person {name: 'Alice', age: 30})")
       result = client.query("MATCH (n:Person) RETURN n.name, n.age")
       for row in result:
           print(f"{row['n.name']}: {row['n.age']}")

Indices and tables
------------------

* :ref:`genindex`
* :ref:`modindex`
* :ref:`search`
