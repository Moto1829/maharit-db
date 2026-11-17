//! リスト述語（all / any / none / single）、複数ラベル、パターン述語、遅延束縛。

use super::*;

#[test]
fn test_predicate_all() {
    let mut graph = Graph::new();
    // Create a sentinel node to drive MATCH
    execute(&mut graph, "CREATE (:T)").unwrap();

    // all(x IN [1,2,3] WHERE x > 0) → true (all positive)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN all(x IN [1, 2, 3] WHERE x > 0)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // all(x IN [1,2,3] WHERE x > 1) → false (1 does not satisfy)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN all(x IN [1, 2, 3] WHERE x > 1)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // all() on empty list → true (vacuously true)
    let result = execute(&mut graph, "MATCH (n:T) RETURN all(x IN [] WHERE x > 0)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));
}

#[test]
fn test_predicate_any() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:T)").unwrap();

    // any(x IN [1,2,3] WHERE x > 2) → true (3 satisfies)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN any(x IN [1, 2, 3] WHERE x > 2)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // any(x IN [1,2,3] WHERE x > 5) → false (none satisfy)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN any(x IN [1, 2, 3] WHERE x > 5)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // any() on empty list → false
    let result = execute(&mut graph, "MATCH (n:T) RETURN any(x IN [] WHERE x > 0)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));
}

#[test]
fn test_predicate_none() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:T)").unwrap();

    // none(x IN [1,2,3] WHERE x > 5) → true (no element satisfies)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN none(x IN [1, 2, 3] WHERE x > 5)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // none(x IN [1,2,3] WHERE x > 0) → false (all satisfy)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN none(x IN [1, 2, 3] WHERE x > 0)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // none() on empty list → true
    let result = execute(&mut graph, "MATCH (n:T) RETURN none(x IN [] WHERE x > 0)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));
}

#[test]
fn test_predicate_single() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:T)").unwrap();

    // single(x IN [1,2,3] WHERE x = 2) → true (exactly one)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x = 2)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // single(x IN [1,2,3] WHERE x > 0) → false (all satisfy, not exactly one)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x > 0)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // single() on empty list → false
    let result = execute(&mut graph, "MATCH (n:T) RETURN single(x IN [] WHERE x = 1)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // single(x IN [1,2,3] WHERE x > 5) → false (zero satisfy)
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN single(x IN [1, 2, 3] WHERE x > 5)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));
}

#[test]
fn test_exists_function() {
    let mut graph = Graph::new();
    // Create a node with email property
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Alice", email: "alice@example.com"})"#,
    )
    .unwrap();
    // Create a node without email property
    execute(&mut graph, r#"CREATE (:Person {name: "Bob"})"#).unwrap();

    // nodes with email should be found by exists(n.email)
    let result = execute(
        &mut graph,
        "MATCH (n:Person) WHERE exists(n.email) RETURN n.name",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );

    // exists() in RETURN context: Alice has email → true
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) RETURN exists(n.email)"#,
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // Bob has no email → false
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Bob"}) RETURN exists(n.email)"#,
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));
}

#[test]
fn test_is_empty_function() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:T)").unwrap();

    // isEmpty on empty list → true
    let result = execute(&mut graph, "MATCH (n:T) RETURN isEmpty([])").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // isEmpty on non-empty list → false
    let result = execute(&mut graph, "MATCH (n:T) RETURN isEmpty([1, 2, 3])").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // isEmpty on empty string → true
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN isEmpty("")"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // isEmpty on non-empty string → false
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN isEmpty("hello")"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));
}

#[test]
fn test_predicate_in_where_clause() {
    let mut graph = Graph::new();
    // Use list predicate in a WHERE clause with MATCH
    execute(&mut graph, r#"CREATE (:Item {val: 5})"#).unwrap();
    execute(&mut graph, r#"CREATE (:Item {val: 15})"#).unwrap();

    // Find items where val equals any number in [4, 5, 6]
    let result = execute(
        &mut graph,
        "MATCH (n:Item) WHERE any(x IN [4, 5, 6] WHERE x = n.val) RETURN n.val",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(5));
}

// ========== Multiple Labels Tests ==========

#[test]
fn test_create_node_with_multiple_labels() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person:Employee {name: "Alice"})"#).unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("Person"), "Should have Person label");
    assert!(node.has_label("Employee"), "Should have Employee label");
    assert_eq!(node.labels.len(), 2);
}

#[test]
fn test_match_multiple_labels_and_condition() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person:Employee {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

    // Only Alice has both Person AND Employee
    let result = execute(&mut graph, r#"MATCH (n:Person:Employee) RETURN n.name"#).unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_match_single_label_on_multilabel_node() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person:Employee {name: "Alice"})"#).unwrap();

    // Node with multiple labels should match on any single label
    let result1 = execute(&mut graph, r#"MATCH (n:Person) RETURN n.name"#).unwrap();
    assert_eq!(result1.row_count(), 1);

    let result2 = execute(&mut graph, r#"MATCH (n:Employee) RETURN n.name"#).unwrap();
    assert_eq!(result2.row_count(), 1);
}

#[test]
fn test_set_label_on_multilabel_node() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    // Add two labels sequentially
    execute(&mut graph, r#"MATCH (n:Person) SET n:Employee"#).unwrap();
    execute(&mut graph, r#"MATCH (n:Person) SET n:Manager"#).unwrap();

    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("Person"));
    assert!(node.has_label("Employee"));
    assert!(node.has_label("Manager"));
    assert_eq!(node.labels.len(), 3);
}

#[test]
fn test_remove_label_from_multilabel_node() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person:Employee:Manager {name: "Alice"})"#,
    )
    .unwrap();

    execute(&mut graph, r#"MATCH (n:Manager) REMOVE n:Manager"#).unwrap();

    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("Person"), "Should still have Person");
    assert!(node.has_label("Employee"), "Should still have Employee");
    assert!(!node.has_label("Manager"), "Manager should be removed");
    assert_eq!(node.labels.len(), 2);
}

#[test]
fn test_labels_function_multiple_labels() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person:Employee {name: "Alice"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:Person) RETURN labels(n)"#).unwrap();
    assert_eq!(result.row_count(), 1);
    match &result.rows[0].columns[0] {
        Value::List(labels) => {
            assert!(labels.contains(&Value::String("Person".to_string())));
            assert!(labels.contains(&Value::String("Employee".to_string())));
            assert_eq!(labels.len(), 2);
        }
        other => panic!("Expected list of labels, got {:?}", other),
    }
}

#[test]
fn test_create_match_three_labels() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:A:B:C {val: 42})"#).unwrap();

    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("A"));
    assert!(node.has_label("B"));
    assert!(node.has_label("C"));
    assert_eq!(node.labels.len(), 3);

    // Matching with all three labels should work
    let result = execute(&mut graph, r#"MATCH (n:A:B:C) RETURN n.val"#).unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(42));

    // Matching with only two labels should also work
    let result2 = execute(&mut graph, r#"MATCH (n:A:B) RETURN n.val"#).unwrap();
    assert_eq!(result2.row_count(), 1);

    // Matching with a non-existent combo should return nothing
    let result3 = execute(&mut graph, r#"MATCH (n:A:D) RETURN n.val"#).unwrap();
    assert_eq!(result3.row_count(), 0);
}

#[test]
fn test_pattern_predicate_basic() {
    let mut graph = Graph::new();
    // Alice knows Bob, Charlie knows nobody
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

    // WHERE (n)-[:KNOWS]->() should return only Alice
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE (n)-[:KNOWS]->() RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_pattern_predicate_not() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

    // WHERE NOT (n)-[:KNOWS]->() should return Bob and Charlie (no outgoing KNOWS)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE NOT (n)-[:KNOWS]->() RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_pattern_predicate_with_bound_variable() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // WHERE (a)-[:KNOWS]->(b) with both a and b bound
    let result = execute(
        &mut graph,
        r#"MATCH (a:Person), (b:Person) WHERE (a)-[:KNOWS]->(b) RETURN a.name, b.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
}

#[test]
fn test_pattern_predicate_with_props() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:WORKS_AT]->(c:Company {name: "ACME"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (b:Person {name: "Bob"})-[:WORKS_AT]->(c2:Company {name: "Other"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE (n)-[:WORKS_AT]->(:Company {name: "ACME"}) RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_pattern_predicate_label_only() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Robot {name: "R2D2"})"#).unwrap();

    // WHERE (n:Person) as standalone pattern predicate
    let result = execute(&mut graph, r#"MATCH (n) WHERE (n:Person) RETURN n.name"#).unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

// ========== Lazy binding (task 59) ==========

#[test]
fn test_lazy_binding_limit_early_termination() {
    // Verify that LIMIT N without ORDER BY terminates early:
    // only N rows should be returned even if more exist.
    let mut graph = Graph::new();
    for i in 0..10 {
        let id = graph.create_node("Item");
        graph
            .get_node_mut(id)
            .unwrap()
            .set_property("n", PropertyValue::Int(i));
    }

    let result = execute(&mut graph, "MATCH (n:Item) RETURN n.n LIMIT 3").unwrap();
    assert_eq!(result.row_count(), 3);
}

#[test]
fn test_lazy_binding_multi_pattern_correctness() {
    // Multi-pattern MATCH should return correct results with lazy per-binding expansion.
    let mut graph = Graph::new();
    let a = graph.create_node("A");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("v", PropertyValue::Int(1));
    let b = graph.create_node("B");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("v", PropertyValue::Int(2));
    graph.create_edge(a, b, "TO").unwrap();

    // Two patterns: node pattern + path pattern
    let result = execute(&mut graph, "MATCH (a:A), (a)-[:TO]->(b:B) RETURN a.v, b.v").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(result.rows[0].columns[1], Value::Int(2));
}

#[test]
fn test_lazy_binding_limit_with_aggregation_unchanged() {
    // Aggregation must still collect all rows (no early cutoff).
    let mut graph = Graph::new();
    for i in 0..5 {
        let id = graph.create_node("N");
        graph
            .get_node_mut(id)
            .unwrap()
            .set_property("v", PropertyValue::Int(i));
    }

    let result = execute(&mut graph, "MATCH (n:N) RETURN count(n) LIMIT 1").unwrap();
    // count must be 5, not 1 (limit does not truncate before aggregation)
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(5));
}

// ========== Task 50: Temporal Types ==========
