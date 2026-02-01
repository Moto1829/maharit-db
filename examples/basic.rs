//! Basic usage example for maharit-db
//!
//! This example demonstrates:
//! - Creating nodes and edges
//! - Using queries (MATCH/WHERE/RETURN)
//! - Setting and getting properties
//! - DELETE operations

use maharit_core::Graph;
use maharit_query::{Executor, Parser};

fn main() {
    println!("=== maharit-db Basic Example ===\n");

    // Create a new graph
    let mut graph = Graph::new();

    // 1. Create nodes using direct API
    println!("1. Creating nodes with direct API...");
    let alice = graph.create_node("Person");
    let bob = graph.create_node("Person");
    let company = graph.create_node("Company");

    // Set properties
    if let Some(node) = graph.get_node_mut(alice) {
        node.set_property("name", "Alice");
        node.set_property("age", 30);
    }
    if let Some(node) = graph.get_node_mut(bob) {
        node.set_property("name", "Bob");
        node.set_property("age", 25);
    }
    if let Some(node) = graph.get_node_mut(company) {
        node.set_property("name", "TechCorp");
        node.set_property("employees", 100);
    }

    println!("   Created {} nodes", graph.node_count());

    // 2. Create edges
    println!("\n2. Creating edges...");
    let knows = graph.create_edge(alice, bob, "KNOWS").unwrap();
    let works_at_alice = graph.create_edge(alice, company, "WORKS_AT").unwrap();
    let works_at_bob = graph.create_edge(bob, company, "WORKS_AT").unwrap();

    // Set edge properties
    if let Some(edge) = graph.get_edge_mut(knows) {
        edge.set_property("since", 2020);
    }
    if let Some(edge) = graph.get_edge_mut(works_at_alice) {
        edge.set_property("role", "Engineer");
    }
    if let Some(edge) = graph.get_edge_mut(works_at_bob) {
        edge.set_property("role", "Designer");
    }

    println!("   Created {} edges", graph.edge_count());

    // 3. Read properties
    println!("\n3. Reading properties...");
    if let Some(node) = graph.get_node(alice) {
        println!(
            "   Alice: name={:?}, age={:?}",
            node.get_property("name"),
            node.get_property("age")
        );
    }

    // 4. Find nodes by label
    println!("\n4. Finding nodes by label...");
    let people = graph.find_nodes_by_label("Person");
    println!("   Found {} Person nodes", people.len());
    for person in &people {
        println!("     - {:?}", person.get_property("name"));
    }

    // 5. Get neighbors
    println!("\n5. Getting neighbors...");
    let alice_neighbors = graph.get_neighbors(alice);
    println!("   Alice has {} neighbors", alice_neighbors.len());

    // 6. Using Cypher-like query language
    println!("\n6. Using query language...");

    // CREATE query
    let create_query = r#"CREATE (c:Person {name: "Charlie", age: 35})"#;
    println!("   Query: {}", create_query);
    let stmt = Parser::new(create_query).unwrap().parse().unwrap();
    let _result = Executor::new(&mut graph).execute(stmt).unwrap();
    println!("   Result: created {} nodes", graph.node_count());

    // MATCH query - find all people
    let match_query = "MATCH (n:Person) RETURN n.name";
    println!("\n   Query: {}", match_query);
    let stmt = Parser::new(match_query).unwrap().parse().unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    println!("   Found {} people:", result.row_count());
    for row in &result.rows {
        println!("     - {}", row.columns[0]);
    }

    // MATCH with WHERE - find people over 28
    let filter_query = "MATCH (n:Person) WHERE n.age > 28 RETURN n.name, n.age";
    println!("\n   Query: {}", filter_query);
    let stmt = Parser::new(filter_query).unwrap().parse().unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    println!("   People over 28:");
    for row in &result.rows {
        println!("     - {}, age: {}", row.columns[0], row.columns[1]);
    }

    // MATCH path pattern - find who knows whom
    let path_query = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name";
    println!("\n   Query: {}", path_query);
    let stmt = Parser::new(path_query).unwrap().parse().unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    println!("   KNOWS relationships:");
    for row in &result.rows {
        println!("     - {} knows {}", row.columns[0], row.columns[1]);
    }

    // 7. DELETE operation
    println!("\n7. DELETE operation...");

    // First, let's see current state
    println!(
        "   Before delete: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Delete a specific node by matching
    let delete_query = r#"MATCH (n:Person {name: "Charlie"}) DETACH DELETE n"#;
    println!("   Query: {}", delete_query);
    let stmt = Parser::new(delete_query).unwrap().parse().unwrap();
    let _ = Executor::new(&mut graph).execute(stmt).unwrap();
    println!(
        "   After delete: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // 8. Direct delete via API
    println!("\n8. Direct delete via API...");
    let eve = graph.create_node("Person");
    println!("   Created temporary node with id={}", eve);
    graph.delete_node(eve);
    println!("   Deleted node, count now: {}", graph.node_count());

    println!("\n=== Example Complete ===");
}
