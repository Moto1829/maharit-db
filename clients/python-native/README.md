# maharit-native

Native Python extension for MaharitDB — in-process graph operations without the TCP server.

## Installation

```bash
pip install maturin
maturin develop  # development build
maturin build --release  # release wheel
```

## Usage

```python
from maharit_native import Graph

g = Graph()

# Create nodes
alice = g.create_node("Person")
bob   = g.create_node("Person")

# Set properties
g.set_node_property(alice, "name", "Alice")
g.set_node_property_int(alice, "age", 30)
g.set_node_property(bob, "name", "Bob")

# Create edge
edge = g.create_edge(alice, bob, "KNOWS")

print(g.node_count())  # 2
print(g.edge_count())  # 1

# Retrieve node
print(g.get_node(alice))
# {'id': 0, 'labels': ['Person'], 'properties': {'name': 'Alice', 'age': 30}}

# Traverse
out_edges = g.get_outgoing_edges(alice)
for eid in out_edges:
    e = g.get_edge(eid)
    print(e)  # {'id': 0, 'label': 'KNOWS', 'from': 0, 'to': 1, 'properties': {}}
```

## vs. TCP client

| | `maharit` (TCP) | `maharit-native` (PyO3) |
|---|---|---|
| Requires server | Yes | No |
| Network overhead | Yes | No (in-process) |
| Query language | Cypher | Python API |
| Use case | Production distributed | Embedded / testing |

## Building

Requires Rust toolchain and `maturin`:

```bash
cargo install maturin
maturin develop --release
```
