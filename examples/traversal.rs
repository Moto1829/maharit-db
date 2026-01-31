//! Graph traversal example for maharit-db
//!
//! This example demonstrates:
//! - BFS (Breadth-First Search) traversal
//! - DFS (Depth-First Search) traversal
//! - Shortest path calculation
//! - Dijkstra's algorithm for weighted paths
//! - A* algorithm with heuristics

use maharit_core::{
    traversal::Direction, AStar, Dijkstra, Graph, PropertyValue,
};

fn main() {
    println!("=== maharit-db Traversal Example ===\n");

    // Create a social network graph
    let mut graph = Graph::new();

    // Create nodes: A -> B -> C -> D
    //               |         |
    //               v         v
    //               E ------> F
    let a = graph.create_node("Person");
    let b = graph.create_node("Person");
    let c = graph.create_node("Person");
    let d = graph.create_node("Person");
    let e = graph.create_node("Person");
    let f = graph.create_node("Person");

    // Set names for easier identification
    for (id, name) in [(a, "A"), (b, "B"), (c, "C"), (d, "D"), (e, "E"), (f, "F")] {
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", name);
        }
    }

    // Create edges with weights
    let edges = [
        (a, b, "KNOWS", 1.0),
        (b, c, "KNOWS", 2.0),
        (c, d, "KNOWS", 1.0),
        (a, e, "KNOWS", 3.0),
        (c, f, "KNOWS", 1.0),
        (e, f, "KNOWS", 2.0),
    ];

    for (from, to, label, weight) in edges {
        let edge_id = graph.create_edge(from, to, label).unwrap();
        if let Some(edge) = graph.get_edge_mut(edge_id) {
            edge.set_property("weight", weight);
        }
    }

    println!("Graph structure:");
    println!("  A -> B -> C -> D");
    println!("  |         |");
    println!("  v         v");
    println!("  E ------> F");
    println!();

    // 1. BFS traversal
    println!("1. BFS (Breadth-First Search) from A:");
    let bfs_results: Vec<_> = graph.traverse(a).bfs().collect();
    for (node_id, depth) in &bfs_results {
        let name = graph
            .get_node(*node_id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| node_id.to_string());
        println!("   depth {}: {}", depth, name);
    }

    // 2. DFS traversal
    println!("\n2. DFS (Depth-First Search) from A:");
    let dfs_results: Vec<_> = graph.traverse(a).dfs().collect();
    for (node_id, depth) in &dfs_results {
        let name = graph
            .get_node(*node_id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| node_id.to_string());
        println!("   depth {}: {}", depth, name);
    }

    // 3. Traversal with max depth
    println!("\n3. BFS with max depth 1 from A:");
    let limited: Vec<_> = graph.traverse(a).max_depth(1).bfs().collect();
    for (node_id, depth) in &limited {
        let name = graph
            .get_node(*node_id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| node_id.to_string());
        println!("   depth {}: {}", depth, name);
    }

    // 4. Traversal with edge filter
    println!("\n4. BFS with edge filter (only KNOWS edges):");
    let filtered: Vec<_> = graph
        .traverse(a)
        .with_edge_filter(|e| e.label == "KNOWS")
        .bfs()
        .collect();
    println!("   Found {} nodes", filtered.len());

    // 5. Bidirectional traversal
    println!("\n5. Bidirectional traversal from C (depth 1):");
    let bidirectional: Vec<_> = graph
        .traverse(c)
        .direction(Direction::Both)
        .max_depth(1)
        .bfs()
        .collect();
    for (node_id, depth) in &bidirectional {
        let name = graph
            .get_node(*node_id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| node_id.to_string());
        println!("   depth {}: {}", depth, name);
    }

    // 6. Shortest path (unweighted, hop count)
    println!("\n6. Shortest path (unweighted) from A to F:");
    if let Some(path) = maharit_core::shortest_path(&graph, a, f) {
        let names: Vec<_> = path
            .nodes
            .iter()
            .map(|id| {
                graph
                    .get_node(*id)
                    .and_then(|n| n.get_property("name"))
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| id.to_string())
            })
            .collect();
        println!("   Path: {} (length: {} hops)", names.join(" -> "), path.len() - 1);
    } else {
        println!("   No path found!");
    }

    // 7. Check if path exists
    println!("\n7. Path existence check:");
    println!("   A to F: {}", maharit_core::has_path(&graph, a, f));
    println!("   F to A: {}", maharit_core::has_path(&graph, f, a)); // No reverse edges

    // 8. Dijkstra's algorithm (weighted shortest path)
    println!("\n8. Dijkstra's algorithm (weighted) from A to D:");
    let dijkstra = Dijkstra::with_weight_property(&graph, "weight");
    if let Some(path) = dijkstra.find_path(a, d) {
        let names: Vec<_> = path
            .nodes
            .iter()
            .map(|id| {
                graph
                    .get_node(*id)
                    .and_then(|n| n.get_property("name"))
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| id.to_string())
            })
            .collect();
        println!(
            "   Path: {} (total weight: {})",
            names.join(" -> "),
            path.total_weight
        );
    }

    // 9. Dijkstra - all distances from A
    println!("\n9. All distances from A (Dijkstra):");
    let distances = dijkstra.distances_from(a);
    let mut sorted_distances: Vec<_> = distances.iter().collect();
    sorted_distances.sort_by(|a, b| a.1.partial_cmp(b.1).unwrap());
    for (node_id, distance) in sorted_distances {
        let name = graph
            .get_node(*node_id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| node_id.to_string());
        println!("   {}: distance = {}", name, distance);
    }

    // 10. A* algorithm
    println!("\n10. A* algorithm from A to F:");
    // Simple heuristic: estimate remaining distance as 0 (becomes Dijkstra-like)
    // In a real scenario, you might use coordinates or other domain knowledge
    let astar = AStar::with_weight_fn(
        &graph,
        |edge| {
            edge.get_property("weight")
                .and_then(|v| match v {
                    PropertyValue::Float(n) => Some(*n),
                    PropertyValue::Int(n) => Some(*n as f64),
                    _ => None,
                })
                .unwrap_or(1.0)
        },
        |_current, _goal| {
            // Admissible heuristic: optimistic estimate
            // Using 0 makes it equivalent to Dijkstra (always admissible)
            0.0
        },
    );

    if let Some(path) = astar.find_path(a, f) {
        let names: Vec<_> = path
            .nodes
            .iter()
            .map(|id| {
                graph
                    .get_node(*id)
                    .and_then(|n| n.get_property("name"))
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| id.to_string())
            })
            .collect();
        println!(
            "   Path: {} (total weight: {})",
            names.join(" -> "),
            path.total_weight
        );
    }

    // 11. Custom A* with informative heuristic
    println!("\n11. A* with custom heuristic (node ID difference):");
    let astar_custom = AStar::with_weight_fn(
        &graph,
        |edge| {
            edge.get_property("weight")
                .and_then(|v| match v {
                    PropertyValue::Float(n) => Some(*n),
                    PropertyValue::Int(n) => Some(*n as f64),
                    _ => None,
                })
                .unwrap_or(1.0)
        },
        |current, goal| {
            // Heuristic based on node ID difference (just for demonstration)
            // This is admissible if IDs are roughly ordered by distance
            let diff = if goal > current {
                goal - current
            } else {
                current - goal
            };
            diff as f64 * 0.5 // Scale factor to keep it admissible
        },
    );

    if let Some(path) = astar_custom.find_path(a, f) {
        let names: Vec<_> = path
            .nodes
            .iter()
            .map(|id| {
                graph
                    .get_node(*id)
                    .and_then(|n| n.get_property("name"))
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| id.to_string())
            })
            .collect();
        println!(
            "   Path: {} (total weight: {})",
            names.join(" -> "),
            path.total_weight
        );
    }

    println!("\n=== Example Complete ===");
}
