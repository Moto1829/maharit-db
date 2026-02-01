//! Visualization example for maharit-db
//!
//! This example demonstrates:
//! - DOT format export for Graphviz
//! - ASCII rendering for terminal display

use maharit_core::Graph;
use maharit_viz::{AsciiRenderer, DotExporter, DotStyle};

fn main() {
    println!("=== maharit-db Visualization Example ===\n");

    // 1. Create a sample graph
    println!("1. Creating sample graph...");
    let mut graph = Graph::new();

    // Create a small social network
    let alice = graph.create_node("Person");
    let bob = graph.create_node("Person");
    let charlie = graph.create_node("Person");
    let diana = graph.create_node("Person");
    let tech_corp = graph.create_node("Company");

    // Set properties
    if let Some(node) = graph.get_node_mut(alice) {
        node.set_property("name", "Alice");
        node.set_property("age", 30);
    }
    if let Some(node) = graph.get_node_mut(bob) {
        node.set_property("name", "Bob");
        node.set_property("age", 25);
    }
    if let Some(node) = graph.get_node_mut(charlie) {
        node.set_property("name", "Charlie");
        node.set_property("age", 35);
    }
    if let Some(node) = graph.get_node_mut(diana) {
        node.set_property("name", "Diana");
        node.set_property("age", 28);
    }
    if let Some(node) = graph.get_node_mut(tech_corp) {
        node.set_property("name", "TechCorp");
        node.set_property("employees", 100);
    }

    // Create relationships
    graph.create_edge(alice, bob, "KNOWS").unwrap();
    graph.create_edge(alice, charlie, "KNOWS").unwrap();
    graph.create_edge(bob, charlie, "KNOWS").unwrap();
    graph.create_edge(charlie, diana, "KNOWS").unwrap();
    graph.create_edge(alice, tech_corp, "WORKS_AT").unwrap();
    graph.create_edge(bob, tech_corp, "WORKS_AT").unwrap();

    println!(
        "   Created: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // 2. DOT format export (default style)
    println!("\n2. DOT format export (default style):");
    println!("   ----------------------------------------");
    let dot = DotExporter::export(&graph);
    println!("{}", dot);

    // 3. DOT format with custom style
    println!("3. DOT format export (custom style):");
    println!("   ----------------------------------------");
    let custom_style = DotStyle {
        top_to_bottom: false, // Left to right layout
        node_shape: "box".to_string(),
        node_color: "lightgreen".to_string(),
        edge_color: "blue".to_string(),
        show_properties: false, // Hide properties
        max_properties: 2,
    };
    let dot_custom = DotExporter::export_with_style(&graph, &custom_style);
    println!("{}", dot_custom);

    // 4. ASCII rendering - full graph
    println!("4. ASCII rendering (full graph):");
    println!("   ----------------------------------------");
    let ascii = AsciiRenderer::render(&graph);
    println!("{}", ascii);

    // 5. ASCII tree from a specific node
    println!("5. ASCII tree from Alice (max depth 2):");
    println!("   ----------------------------------------");
    let tree = AsciiRenderer::render_tree(&graph, alice, 2);
    println!("{}", tree);

    // 6. ASCII tree from Bob
    println!("6. ASCII tree from Bob (max depth 3):");
    println!("   ----------------------------------------");
    let tree_bob = AsciiRenderer::render_tree(&graph, bob, 3);
    println!("{}", tree_bob);

    // 7. ASCII layers (BFS visualization)
    println!("7. ASCII layers from Alice (max depth 3):");
    println!("   ----------------------------------------");
    let layers = AsciiRenderer::render_layers(&graph, alice, 3);
    println!("{}", layers);

    // 8. Empty graph rendering
    println!("8. Empty graph rendering:");
    println!("   ----------------------------------------");
    let empty_graph = Graph::new();
    let empty_ascii = AsciiRenderer::render(&empty_graph);
    println!("   {}", empty_ascii);

    // 9. Circular graph detection
    println!("\n9. Circular graph detection:");
    println!("   ----------------------------------------");
    let mut circular_graph = Graph::new();
    let n1 = circular_graph.create_node("Node");
    let n2 = circular_graph.create_node("Node");
    let n3 = circular_graph.create_node("Node");

    if let Some(node) = circular_graph.get_node_mut(n1) {
        node.set_property("name", "N1");
    }
    if let Some(node) = circular_graph.get_node_mut(n2) {
        node.set_property("name", "N2");
    }
    if let Some(node) = circular_graph.get_node_mut(n3) {
        node.set_property("name", "N3");
    }

    circular_graph.create_edge(n1, n2, "NEXT").unwrap();
    circular_graph.create_edge(n2, n3, "NEXT").unwrap();
    circular_graph.create_edge(n3, n1, "NEXT").unwrap(); // Creates a cycle

    let circular_tree = AsciiRenderer::render_tree(&circular_graph, n1, 5);
    println!("{}", circular_tree);

    // 10. DOT output for complex graph
    println!("10. DOT for circular graph:");
    println!("    ----------------------------------------");
    let circular_dot = DotExporter::export(&circular_graph);
    println!("{}", circular_dot);

    // 11. Large graph visualization tips
    println!("11. Large graph tips:");
    println!("    ----------------------------------------");
    println!("    For large graphs, consider:");
    println!("    - Using max_depth to limit traversal");
    println!("    - Filtering nodes/edges before visualization");
    println!("    - Exporting DOT and using Graphviz tools:");
    println!("      $ dot -Tpng graph.dot -o graph.png");
    println!("      $ dot -Tsvg graph.dot -o graph.svg");
    println!("    - Using neato for undirected layouts:");
    println!("      $ neato -Tpng graph.dot -o graph.png");

    // 12. Custom DOT style showcase
    println!("\n12. DOT style options showcase:");
    println!("    ----------------------------------------");

    let styles = [
        ("Default", DotStyle::default()),
        (
            "Minimal",
            DotStyle {
                top_to_bottom: true,
                node_shape: "circle".to_string(),
                node_color: "white".to_string(),
                edge_color: "black".to_string(),
                show_properties: false,
                max_properties: 0,
            },
        ),
        (
            "Rich",
            DotStyle {
                top_to_bottom: false,
                node_shape: "record".to_string(),
                node_color: "lightyellow".to_string(),
                edge_color: "darkgray".to_string(),
                show_properties: true,
                max_properties: 5,
            },
        ),
    ];

    for (name, style) in &styles {
        println!("\n    Style: {}", name);
        println!("      node_shape: {}", style.node_shape);
        println!("      node_color: {}", style.node_color);
        println!("      edge_color: {}", style.edge_color);
        println!("      show_properties: {}", style.show_properties);
        println!(
            "      direction: {}",
            if style.top_to_bottom {
                "top-to-bottom"
            } else {
                "left-to-right"
            }
        );
    }

    println!("\n=== Example Complete ===");
}
