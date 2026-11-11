---
title: Cycle detection
parent: Graph algorithms
nav_order: 5
source_commit: f427cc91
---

# Cycle detection

MaharitDB provides three functions for detecting cycles in a graph. Use them to validate dependency graphs, check that a graph is a directed acyclic graph (DAG), and get a topological order. They are part of the Rust API of `maharit-core` and follow relationship direction.

## Overview

| Function | Purpose | Complexity |
|------|------|--------|
| `has_cycle()` | Tell whether there is a cycle | O(V + E) |
| `find_cycles()` | List the cycles | O(V + E) |
| `topological_sort()` | Get a topological order | O(V + E) |

Typical uses:

- **Validating dependencies**: detect circular dependencies in a package manager
- **Task scheduling**: confirm that an execution order has no contradictions
- **Data pipelines**: make sure a processing flow has no loops
- **Schema design**: confirm that foreign-key references have no cycles

## has_cycle()

Checks for a cycle with depth-first search (DFS). It detects back edges using a recursion stack (`rec_stack`) and returns `true` as soon as it finds the first cycle.

```rust
pub fn has_cycle(graph: &Graph) -> bool
```

### Example in Rust

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn main() {
    let mut graph = Graph::new();

    let a = node(&mut graph, "Task", "A");
    let b = node(&mut graph, "Task", "B");
    let c = node(&mut graph, "Task", "C");

    // A -> B -> C (no cycle)
    graph.create_edge(a, b, "DEPENDS_ON").unwrap();
    graph.create_edge(b, c, "DEPENDS_ON").unwrap();

    println!("DAG: {}", !algorithms::has_cycle(&graph)); // DAG: true

    // Add C -> A to create a cycle
    graph.create_edge(c, a, "DEPENDS_ON").unwrap();

    println!("DAG: {}", !algorithms::has_cycle(&graph)); // DAG: false
}
```

## find_cycles()

Lists the cycles in the graph. It uses back-edge detection in DFS and returns lists of the node IDs that form each cycle.

```rust
pub fn find_cycles(graph: &Graph) -> Vec<Vec<NodeId>>
```

### Return value

Returns `Vec<Vec<NodeId>>`. Each inner vector lists the node IDs that form one cycle.

Example: for the cycle `A -> B -> C -> A`, a vector like `[NodeId_A, NodeId_B, NodeId_C]` is returned (the start is not repeated at the end).

If there is no cycle, an empty `Vec` is returned.

### Example in Rust

```rust
use maharit_core::{Graph, NodeId, PropertyValue, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn name(graph: &Graph, id: NodeId) -> String {
    match graph.get_node(id).and_then(|n| n.get_property("name")) {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => id.to_string(),
    }
}

fn main() {
    let mut graph = Graph::new();

    let pkg_a = node(&mut graph, "Package", "pkg-a");
    let pkg_b = node(&mut graph, "Package", "pkg-b");
    let pkg_c = node(&mut graph, "Package", "pkg-c");
    let pkg_d = node(&mut graph, "Package", "pkg-d");

    // pkg-a -> pkg-b -> pkg-c -> pkg-b (circular dependency)
    graph.create_edge(pkg_a, pkg_b, "REQUIRES").unwrap();
    graph.create_edge(pkg_b, pkg_c, "REQUIRES").unwrap();
    graph.create_edge(pkg_c, pkg_b, "REQUIRES").unwrap();
    // pkg-a -> pkg-d (a normal dependency)
    graph.create_edge(pkg_a, pkg_d, "REQUIRES").unwrap();

    let cycles = algorithms::find_cycles(&graph);

    if cycles.is_empty() {
        println!("No circular dependencies");
    } else {
        println!("Found {} circular dependencies:", cycles.len());
        for (i, cycle) in cycles.iter().enumerate() {
            let names: Vec<String> = cycle
                .iter()
                .map(|&node_id| {
                    name(&graph, node_id)
                })
                .collect();
            println!("  Cycle {}: {}", i + 1, names.join(" -> "));
        }
    }
}
```

Example output:

```
Found 1 circular dependencies:
  Cycle 1: pkg-b -> pkg-c
```

## topological_sort()

Performs a topological sort with Kahn's algorithm (BFS on in-degrees). The order is meaningful only when the graph is a DAG (directed acyclic graph).

```rust
pub fn topological_sort(graph: &Graph) -> Option<Vec<NodeId>>
```

### Return value

- `Some(Vec<NodeId>)`: the graph has no cycle; node IDs are listed in topological order, with each relationship's start node before its end node
- `None`: the graph has a cycle, so no topological order exists

### Example in Rust

```rust
use maharit_core::{Graph, NodeId, PropertyValue, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn name(graph: &Graph, id: NodeId) -> String {
    match graph.get_node(id).and_then(|n| n.get_property("name")) {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => id.to_string(),
    }
}

fn main() {
    let mut graph = Graph::new();

    // Dependency graph of build tasks
    let install_deps = node(&mut graph, "Task", "install_deps");
    let compile     = node(&mut graph, "Task", "compile");
    let test        = node(&mut graph, "Task", "test");
    let package     = node(&mut graph, "Task", "package");
    let deploy      = node(&mut graph, "Task", "deploy");

    graph.create_edge(install_deps, compile, "BEFORE").unwrap();
    graph.create_edge(compile, test, "BEFORE").unwrap();
    graph.create_edge(test, package, "BEFORE").unwrap();
    graph.create_edge(package, deploy, "BEFORE").unwrap();

    match algorithms::topological_sort(&graph) {
        Some(order) => {
            let names: Vec<String> = order
                .iter()
                .map(|&node_id| {
                    name(&graph, node_id)
                })
                .collect();
            println!("Execution order: {}", names.join(" -> "));
        }
        None => {
            eprintln!("Error: the task dependency graph has a cycle");
        }
    }
}
```

Example output:

```
Execution order: install_deps -> compile -> test -> package -> deploy
```

With a cycle:

```rust
// Add a cycle
graph.create_edge(deploy, compile, "BEFORE").unwrap();

match algorithms::topological_sort(&graph) {
    Some(_) => println!("It is a DAG"),
    None    => println!("A cycle was detected"), // This one is printed
}
```

## Use case examples

### Resolving package dependencies

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn resolve_install_order(graph: &Graph) -> Result<Vec<NodeId>, String> {
    algorithms::topological_sort(graph)
        .ok_or_else(|| "There is a circular dependency; cannot determine the install order.".to_string())
}
```

### Validating a task dependency graph

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn validate_pipeline(graph: &Graph) -> Result<(), Vec<Vec<NodeId>>> {
    let cycles = algorithms::find_cycles(graph);
    if cycles.is_empty() {
        Ok(())
    } else {
        Err(cycles)
    }
}
```

### Checking a CI/CD pipeline

```rust
use maharit_core::{Graph, algorithms};

fn check_pipeline_integrity(graph: &Graph) -> bool {
    // No cycle means a topological order exists
    !algorithms::has_cycle(graph)
}
```

## Performance

Every function runs in **O(V + E)** time (V = nodes, E = relationships).

| Function | Time | Space | Notes |
|------|-----------|-----------|------|
| `has_cycle()` | O(V + E) | O(V) | Stops at the first cycle found |
| `find_cycles()` | O(V + E) | O(V) | Does not stop early, since it lists every cycle |
| `topological_sort()` | O(V + E) | O(V) | Kahn's algorithm (BFS on in-degrees) |

If you only need to know whether a cycle exists, `has_cycle()` is the most efficient. Use `find_cycles()` when you need the cycles themselves.

`NodeId` is a type alias for `u64`. Large graphs (millions of nodes) are processed in linear time, but `find_cycles()` can use a lot of memory for its result when there are many cycles.
