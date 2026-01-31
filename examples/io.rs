//! Import/Export example for maharit-db
//!
//! This example demonstrates:
//! - CSV import and export for nodes and edges
//! - JSON import and export for entire graphs

use std::fs;

use maharit_core::Graph;
use maharit_io::{CsvExporter, CsvImporter, JsonExporter, JsonImporter};

fn main() {
    println!("=== maharit-db I/O Example ===\n");

    // Clean up any previous test files
    let csv_nodes_path = "/tmp/maharit_nodes.csv";
    let csv_edges_path = "/tmp/maharit_edges.csv";
    let json_path = "/tmp/maharit_graph.json";
    for path in [csv_nodes_path, csv_edges_path, json_path] {
        let _ = fs::remove_file(path);
    }

    // 1. Create a sample graph
    println!("1. Creating sample graph...");
    let mut graph = Graph::new();

    // Create nodes
    let alice = graph.create_node("Person");
    let bob = graph.create_node("Person");
    let charlie = graph.create_node("Person");
    let tech_corp = graph.create_node("Company");
    let startup = graph.create_node("Company");

    // Set node properties
    if let Some(node) = graph.get_node_mut(alice) {
        node.set_property("name", "Alice");
        node.set_property("age", 30);
        node.set_property("city", "Tokyo");
    }
    if let Some(node) = graph.get_node_mut(bob) {
        node.set_property("name", "Bob");
        node.set_property("age", 25);
        node.set_property("city", "Osaka");
    }
    if let Some(node) = graph.get_node_mut(charlie) {
        node.set_property("name", "Charlie");
        node.set_property("age", 35);
        node.set_property("city", "Kyoto");
    }
    if let Some(node) = graph.get_node_mut(tech_corp) {
        node.set_property("name", "TechCorp");
        node.set_property("employees", 100);
    }
    if let Some(node) = graph.get_node_mut(startup) {
        node.set_property("name", "StartupInc");
        node.set_property("employees", 10);
    }

    // Create edges
    let knows1 = graph.create_edge(alice, bob, "KNOWS").unwrap();
    let knows2 = graph.create_edge(bob, charlie, "KNOWS").unwrap();
    let knows3 = graph.create_edge(alice, charlie, "KNOWS").unwrap();
    let works1 = graph.create_edge(alice, tech_corp, "WORKS_AT").unwrap();
    let works2 = graph.create_edge(bob, tech_corp, "WORKS_AT").unwrap();
    let works3 = graph.create_edge(charlie, startup, "WORKS_AT").unwrap();

    // Set edge properties
    if let Some(edge) = graph.get_edge_mut(knows1) {
        edge.set_property("since", 2018);
    }
    if let Some(edge) = graph.get_edge_mut(knows2) {
        edge.set_property("since", 2020);
    }
    if let Some(edge) = graph.get_edge_mut(knows3) {
        edge.set_property("since", 2019);
    }
    if let Some(edge) = graph.get_edge_mut(works1) {
        edge.set_property("role", "Engineer");
        edge.set_property("years", 5);
    }
    if let Some(edge) = graph.get_edge_mut(works2) {
        edge.set_property("role", "Designer");
        edge.set_property("years", 3);
    }
    if let Some(edge) = graph.get_edge_mut(works3) {
        edge.set_property("role", "CTO");
        edge.set_property("years", 2);
    }

    println!("   Created: {} nodes, {} edges", graph.node_count(), graph.edge_count());

    // 2. Export to CSV
    println!("\n2. Exporting to CSV...");
    {
        // Export nodes
        let nodes_file = fs::File::create(csv_nodes_path).unwrap();
        let nodes_count = CsvExporter::export_nodes(&graph, nodes_file).unwrap();
        println!("   Exported {} nodes to {}", nodes_count, csv_nodes_path);

        // Export edges
        let edges_file = fs::File::create(csv_edges_path).unwrap();
        let edges_count = CsvExporter::export_edges(&graph, edges_file).unwrap();
        println!("   Exported {} edges to {}", edges_count, csv_edges_path);

        // Show CSV content
        println!("\n   Nodes CSV content:");
        let content = fs::read_to_string(csv_nodes_path).unwrap();
        for line in content.lines().take(4) {
            println!("     {}", line);
        }
        if content.lines().count() > 4 {
            println!("     ...");
        }

        println!("\n   Edges CSV content:");
        let content = fs::read_to_string(csv_edges_path).unwrap();
        for line in content.lines().take(4) {
            println!("     {}", line);
        }
        if content.lines().count() > 4 {
            println!("     ...");
        }
    }

    // 3. Import from CSV
    println!("\n3. Importing from CSV...");
    {
        let mut imported_graph = Graph::new();

        // Import nodes
        let nodes_file = fs::File::open(csv_nodes_path).unwrap();
        let node_stats = CsvImporter::import_nodes(&mut imported_graph, nodes_file).unwrap();
        println!("   Imported {} nodes", node_stats.nodes_imported);

        // Import edges (using the ID map from node import)
        let edges_file = fs::File::open(csv_edges_path).unwrap();
        let edge_stats =
            CsvImporter::import_edges(&mut imported_graph, edges_file, &node_stats.id_map).unwrap();
        println!("   Imported {} edges", edge_stats.edges_imported);

        println!(
            "   Imported graph: {} nodes, {} edges",
            imported_graph.node_count(),
            imported_graph.edge_count()
        );

        // Verify
        let people = imported_graph.find_nodes_by_label("Person");
        println!("   Verified: {} Person nodes", people.len());
    }

    // 4. Export to JSON
    println!("\n4. Exporting to JSON...");
    {
        let json_file = fs::File::create(json_path).unwrap();
        JsonExporter::export(&graph, json_file).unwrap();
        println!("   Exported to {}", json_path);

        // Show JSON content
        println!("\n   JSON content (truncated):");
        let content = fs::read_to_string(json_path).unwrap();
        let lines: Vec<_> = content.lines().collect();
        for line in lines.iter().take(15) {
            println!("     {}", line);
        }
        if lines.len() > 15 {
            println!("     ...");
        }
    }

    // 5. Import from JSON
    println!("\n5. Importing from JSON...");
    {
        let mut imported_graph = Graph::new();

        let json_file = fs::File::open(json_path).unwrap();
        let stats = JsonImporter::import(&mut imported_graph, json_file).unwrap();

        println!("   Imported {} nodes, {} edges", stats.nodes_imported, stats.edges_imported);
        println!(
            "   Imported graph: {} nodes, {} edges",
            imported_graph.node_count(),
            imported_graph.edge_count()
        );

        // Verify
        let companies = imported_graph.find_nodes_by_label("Company");
        println!("   Verified: {} Company nodes", companies.len());
        for company in &companies {
            println!("     - {:?}", company.get_property("name"));
        }
    }

    // 6. Export to JSON string (in memory)
    println!("\n6. Export to JSON string (in memory)...");
    {
        let json_str = JsonExporter::export_to_string(&graph).unwrap();
        println!("   JSON string length: {} bytes", json_str.len());
    }

    // 7. Import from JSON string
    println!("\n7. Import from JSON string...");
    {
        let json_str = r#"{
            "nodes": [
                {"id": "n1", "label": "City", "properties": {"name": "Paris", "country": "France"}},
                {"id": "n2", "label": "City", "properties": {"name": "London", "country": "UK"}},
                {"id": "n3", "label": "City", "properties": {"name": "Berlin", "country": "Germany"}}
            ],
            "edges": [
                {"from": "n1", "to": "n2", "type": "CONNECTED_TO", "properties": {"distance": 344}},
                {"from": "n2", "to": "n3", "type": "CONNECTED_TO", "properties": {"distance": 932}}
            ]
        }"#;

        let mut city_graph = Graph::new();
        let stats = JsonImporter::import(&mut city_graph, json_str.as_bytes()).unwrap();

        println!("   Imported {} nodes, {} edges", stats.nodes_imported, stats.edges_imported);

        let cities = city_graph.find_nodes_by_label("City");
        println!("   Cities:");
        for city in &cities {
            println!(
                "     - {:?} ({:?})",
                city.get_property("name"),
                city.get_property("country")
            );
        }
    }

    // 8. Roundtrip verification
    println!("\n8. Roundtrip verification...");
    {
        // Original -> JSON -> New Graph
        let json_str = JsonExporter::export_to_string(&graph).unwrap();
        let mut roundtrip_graph = Graph::new();
        JsonImporter::import(&mut roundtrip_graph, json_str.as_bytes()).unwrap();

        let original_nodes = graph.node_count();
        let roundtrip_nodes = roundtrip_graph.node_count();
        let original_edges = graph.edge_count();
        let roundtrip_edges = roundtrip_graph.edge_count();

        println!("   Original:  {} nodes, {} edges", original_nodes, original_edges);
        println!("   Roundtrip: {} nodes, {} edges", roundtrip_nodes, roundtrip_edges);
        println!(
            "   Match: {}",
            original_nodes == roundtrip_nodes && original_edges == roundtrip_edges
        );
    }

    // 9. Clean up
    println!("\n9. Cleaning up...");
    fs::remove_file(csv_nodes_path).ok();
    fs::remove_file(csv_edges_path).ok();
    fs::remove_file(json_path).ok();
    println!("   Removed test files");

    println!("\n=== Example Complete ===");
}
