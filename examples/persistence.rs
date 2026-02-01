//! Persistence example for maharit-db
//!
//! This example demonstrates:
//! - Saving and loading graphs to/from files
//! - Write-Ahead Log (WAL) for durability
//! - Recovery from WAL

use std::fs;

use maharit_core::Graph;
use maharit_storage::{PersistentStorage, RecordPayload, RecordType, Wal};

fn main() {
    println!("=== maharit-db Persistence Example ===\n");

    // Clean up any previous test files
    let db_path = "/tmp/maharit_example.db";
    let wal_path = "/tmp/maharit_example.wal";
    let _ = fs::remove_file(db_path);
    let _ = fs::remove_file(wal_path);

    // 1. Create a graph and save it
    println!("1. Creating and saving a graph...");
    {
        let mut graph = Graph::new();

        // Create nodes
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");
        let company = graph.create_node("Company");

        // Set properties
        if let Some(node) = graph.get_node_mut(alice) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
            node.set_property("active", true);
        }
        if let Some(node) = graph.get_node_mut(bob) {
            node.set_property("name", "Bob");
            node.set_property("age", 25);
        }
        if let Some(node) = graph.get_node_mut(company) {
            node.set_property("name", "TechCorp");
        }

        // Create edges
        let edge_id = graph.create_edge(alice, bob, "KNOWS").unwrap();
        if let Some(edge) = graph.get_edge_mut(edge_id) {
            edge.set_property("since", 2020);
        }
        graph.create_edge(alice, company, "WORKS_AT").unwrap();
        graph.create_edge(bob, company, "WORKS_AT").unwrap();

        println!(
            "   Created graph: {} nodes, {} edges",
            graph.node_count(),
            graph.edge_count()
        );

        // Save to file
        PersistentStorage::save(&graph, db_path).unwrap();
        println!("   Saved to: {}", db_path);
    }

    // 2. Load the graph from file
    println!("\n2. Loading graph from file...");
    {
        let loaded_graph = PersistentStorage::load(db_path).unwrap();

        println!(
            "   Loaded graph: {} nodes, {} edges",
            loaded_graph.node_count(),
            loaded_graph.edge_count()
        );

        // Verify data
        let people = loaded_graph.find_nodes_by_label("Person");
        println!("   Found {} Person nodes:", people.len());
        for person in &people {
            println!("     - {:?}", person.get_property("name"));
        }

        let companies = loaded_graph.find_nodes_by_label("Company");
        println!("   Found {} Company nodes", companies.len());

        let edges = loaded_graph.find_edges_by_type("KNOWS");
        println!("   Found {} KNOWS edges", edges.len());
    }

    // 3. Using Write-Ahead Log (WAL)
    println!("\n3. Using Write-Ahead Log (WAL)...");
    {
        // Open WAL file
        let mut wal = Wal::open(wal_path).unwrap();

        // Log some operations
        println!("   Logging operations to WAL...");

        // Create node operations
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "Person".to_string(),
            },
        )
        .unwrap();

        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 1,
                label: "Person".to_string(),
            },
        )
        .unwrap();

        // Set property operations
        wal.append(
            RecordType::SetNodeProperty,
            RecordPayload::SetNodeProperty {
                node_id: 0,
                key: "name".to_string(),
                value: "Eve".into(),
            },
        )
        .unwrap();

        wal.append(
            RecordType::SetNodeProperty,
            RecordPayload::SetNodeProperty {
                node_id: 1,
                key: "name".to_string(),
                value: "Frank".into(),
            },
        )
        .unwrap();

        // Create edge operation
        wal.append(
            RecordType::CreateEdge,
            RecordPayload::CreateEdge {
                edge_id: 0,
                from: 0,
                to: 1,
                label: "FRIENDS_WITH".to_string(),
            },
        )
        .unwrap();

        // Create a checkpoint
        let checkpoint_lsn = wal.checkpoint().unwrap();
        println!("   Created checkpoint at LSN: {}", checkpoint_lsn);

        // Sync to disk
        wal.sync().unwrap();
        println!("   WAL synced to disk");
        println!("   Current LSN: {}", wal.current_lsn());
    }

    // 4. Recovery from WAL
    println!("\n4. Recovering graph from WAL...");
    {
        // Open WAL (simulating restart)
        let wal = Wal::open(wal_path).unwrap();

        // Create empty graph
        let mut recovered_graph = Graph::new();

        // Recover from WAL
        let last_lsn = wal.recover(&mut recovered_graph).unwrap();

        println!("   Recovered to LSN: {}", last_lsn);
        println!(
            "   Recovered graph: {} nodes, {} edges",
            recovered_graph.node_count(),
            recovered_graph.edge_count()
        );

        // Verify recovered data
        for node in recovered_graph.nodes() {
            println!(
                "     Node {}: label={}, name={:?}",
                node.id,
                node.label,
                node.get_property("name")
            );
        }

        for edge in recovered_graph.edges() {
            println!(
                "     Edge {}: {} --[:{}]--> {}",
                edge.id, edge.from, edge.label, edge.to
            );
        }
    }

    // 5. Demonstrating multiple WAL sessions
    println!("\n5. Multiple WAL sessions...");
    {
        // Clean up previous WAL and start fresh
        let _ = fs::remove_file(wal_path);

        // Session 1: Create initial data
        {
            let mut wal = Wal::open(wal_path).unwrap();
            println!("   Session 1: Creating initial data...");

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "User".to_string(),
                },
            )
            .unwrap();

            wal.append(
                RecordType::SetNodeProperty,
                RecordPayload::SetNodeProperty {
                    node_id: 0,
                    key: "username".to_string(),
                    value: "admin".into(),
                },
            )
            .unwrap();

            wal.sync().unwrap();
            println!("   Session 1 LSN: {}", wal.current_lsn());
        }

        // Session 2: Recover and verify
        {
            let wal = Wal::open(wal_path).unwrap();
            let mut graph = Graph::new();
            let lsn = wal.recover(&mut graph).unwrap();

            println!(
                "   Session 2: Recovered {} nodes at LSN {}",
                graph.node_count(),
                lsn
            );
            for node in graph.nodes() {
                println!(
                    "     - {} ({})",
                    node.label,
                    node.get_property("username").unwrap()
                );
            }
        }
    }

    // 6. Clean up
    println!("\n6. Cleaning up...");
    fs::remove_file(db_path).ok();
    fs::remove_file(wal_path).ok();
    println!("   Removed test files");

    println!("\n=== Example Complete ===");
}
