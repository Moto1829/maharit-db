//! Graph algorithms example for maharit-db
//!
//! This example demonstrates:
//! - Centrality measures (degree, closeness, betweenness)
//! - PageRank algorithm
//! - Connected and strongly connected components
//! - Cycle detection and topological sort

use maharit_core::{
    DegreeCentrality, Graph, PageRankConfig, betweenness_centrality, closeness_centrality,
    connected_components, find_cycles, has_cycle, pagerank, strongly_connected_components,
    topological_sort,
};

fn main() {
    println!("=== maharit-db Graph Algorithms Example ===\n");

    // 1. Create a sample graph (web-like structure for PageRank demo)
    println!("1. Creating sample graph...");
    let mut graph = Graph::new();

    // Create nodes representing web pages
    let home = graph.create_node("Page");
    let about = graph.create_node("Page");
    let products = graph.create_node("Page");
    let blog = graph.create_node("Page");
    let contact = graph.create_node("Page");

    // Set names
    for (id, name) in [
        (home, "Home"),
        (about, "About"),
        (products, "Products"),
        (blog, "Blog"),
        (contact, "Contact"),
    ] {
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", name);
        }
    }

    // Create links (edges)
    // Home links to all pages
    graph.create_edge(home, about, "LINKS_TO").unwrap();
    graph.create_edge(home, products, "LINKS_TO").unwrap();
    graph.create_edge(home, blog, "LINKS_TO").unwrap();
    graph.create_edge(home, contact, "LINKS_TO").unwrap();

    // About links back to home and to contact
    graph.create_edge(about, home, "LINKS_TO").unwrap();
    graph.create_edge(about, contact, "LINKS_TO").unwrap();

    // Products links to blog and home
    graph.create_edge(products, blog, "LINKS_TO").unwrap();
    graph.create_edge(products, home, "LINKS_TO").unwrap();

    // Blog links to products and home
    graph.create_edge(blog, products, "LINKS_TO").unwrap();
    graph.create_edge(blog, home, "LINKS_TO").unwrap();

    // Contact links to home
    graph.create_edge(contact, home, "LINKS_TO").unwrap();

    println!(
        "   Created {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Helper to get node name
    let get_name = |id: u64| -> String {
        graph
            .get_node(id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| id.to_string())
    };

    // 2. Degree Centrality
    println!("\n2. Degree Centrality:");
    let degree = DegreeCentrality::compute(&graph);
    println!("   Node         | In | Out | Total");
    println!("   -------------|----|----|-------");
    for node in graph.nodes() {
        println!(
            "   {:12} | {:2} | {:2} | {:2}",
            get_name(node.id),
            degree.in_degree[&node.id],
            degree.out_degree[&node.id],
            degree.total_degree[&node.id]
        );
    }

    // Normalized degree centrality
    println!("\n   Normalized degree centrality:");
    let normalized = degree.normalized(&graph);
    let mut sorted: Vec<_> = normalized.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    for (id, score) in sorted {
        println!("   {:12}: {:.4}", get_name(*id), score);
    }

    // 3. Closeness Centrality
    println!("\n3. Closeness Centrality:");
    let closeness = closeness_centrality(&graph);
    let mut sorted: Vec<_> = closeness.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    for (id, score) in sorted {
        println!("   {:12}: {:.4}", get_name(*id), score);
    }

    // 4. Betweenness Centrality
    println!("\n4. Betweenness Centrality:");
    let betweenness = betweenness_centrality(&graph);
    let mut sorted: Vec<_> = betweenness.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    for (id, score) in sorted {
        println!("   {:12}: {:.4}", get_name(*id), score);
    }

    // 5. PageRank
    println!("\n5. PageRank:");
    let config = PageRankConfig {
        damping: 0.85,
        max_iterations: 100,
        tolerance: 1e-6,
    };
    let pr_result = pagerank(&graph, &config);
    println!(
        "   Converged: {} (iterations: {})",
        pr_result.converged, pr_result.iterations
    );
    println!("\n   PageRank scores:");
    let mut sorted: Vec<_> = pr_result.scores.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    for (id, score) in sorted {
        println!("   {:12}: {:.6}", get_name(*id), score);
    }

    // 6. Connected Components
    println!("\n6. Connected Components:");
    let components = connected_components(&graph);
    println!("   Found {} component(s)", components.len());
    for (i, component) in components.iter().enumerate() {
        let names: Vec<_> = component.iter().map(|&id| get_name(id)).collect();
        println!("   Component {}: {}", i + 1, names.join(", "));
    }

    // 7. Strongly Connected Components
    println!("\n7. Strongly Connected Components:");
    let sccs = strongly_connected_components(&graph);
    println!("   Found {} SCC(s)", sccs.len());
    for (i, scc) in sccs.iter().enumerate() {
        let names: Vec<_> = scc.iter().map(|&id| get_name(id)).collect();
        println!("   SCC {}: {}", i + 1, names.join(", "));
    }

    // 8. Cycle Detection
    println!("\n8. Cycle Detection:");
    let has_cycles = has_cycle(&graph);
    println!("   Graph has cycle: {}", has_cycles);

    if has_cycles {
        let cycles = find_cycles(&graph);
        println!("   Found {} cycle(s)", cycles.len());
        for (i, cycle) in cycles.iter().enumerate().take(3) {
            let names: Vec<_> = cycle.iter().map(|&id| get_name(id)).collect();
            println!("   Cycle {}: {} -> ...", i + 1, names.join(" -> "));
        }
    }

    // 9. Topological Sort
    println!("\n9. Topological Sort:");
    match topological_sort(&graph) {
        Some(sorted) => {
            let names: Vec<_> = sorted.iter().map(|&id| get_name(id)).collect();
            println!("   Order: {}", names.join(" -> "));
        }
        None => {
            println!("   Cannot topologically sort (graph has cycle)");
        }
    }

    // 10. DAG example for topological sort
    println!("\n10. DAG Example (for topological sort):");
    let mut dag = Graph::new();
    let task_a = dag.create_node("Task");
    let task_b = dag.create_node("Task");
    let task_c = dag.create_node("Task");
    let task_d = dag.create_node("Task");
    let task_e = dag.create_node("Task");

    for (id, name) in [
        (task_a, "A: Design"),
        (task_b, "B: Implement"),
        (task_c, "C: Test"),
        (task_d, "D: Review"),
        (task_e, "E: Deploy"),
    ] {
        if let Some(node) = dag.get_node_mut(id) {
            node.set_property("name", name);
        }
    }

    // Dependencies: A -> B -> C -> E, A -> D -> E
    dag.create_edge(task_a, task_b, "DEPENDS_ON").unwrap();
    dag.create_edge(task_b, task_c, "DEPENDS_ON").unwrap();
    dag.create_edge(task_c, task_e, "DEPENDS_ON").unwrap();
    dag.create_edge(task_a, task_d, "DEPENDS_ON").unwrap();
    dag.create_edge(task_d, task_e, "DEPENDS_ON").unwrap();

    let get_dag_name = |id: u64| -> String {
        dag.get_node(id)
            .and_then(|n| n.get_property("name"))
            .map(|v| format!("{}", v))
            .unwrap_or_else(|| id.to_string())
    };

    println!("   Has cycle: {}", has_cycle(&dag));

    if let Some(sorted) = topological_sort(&dag) {
        println!("   Execution order:");
        for (i, id) in sorted.iter().enumerate() {
            println!("     {}. {}", i + 1, get_dag_name(*id));
        }
    }

    println!("\n=== Example Complete ===");
}
