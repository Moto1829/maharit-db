//! CREATE / SET / MERGE / REMOVE / UNWIND の書き込み。

use super::*;

#[test]
fn test_match_create_relationship() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    assert_eq!(graph.edge_count(), 1);
    let edge = graph.edges().next().unwrap();
    assert_eq!(edge.label, "KNOWS");
    // created_nodes should be 0 (reusing existing), created_edges should be 1
    assert_eq!(result.rows[0].columns[1], Value::Int(1));
}

#[test]
fn test_match_create_new_node_and_relationship() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}) CREATE (a)-[:OWNS]->(c:Car {model: "Tesla"})"#,
    )
    .unwrap();

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.edge_count(), 1);

    let car = graph
        .nodes()
        .find(|n| n.has_label("Car"))
        .expect("Car node should exist");
    assert_eq!(
        car.get_property("model"),
        Some(&PropertyValue::String("Tesla".to_string()))
    );
}

#[test]
fn test_match_create_with_where() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice", age: 25})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob", age: 15})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (a:Person) WHERE a.age > 20 CREATE (a)-[:MEMBER_OF]->(g:Group {name: "Adults"})"#,
    )
    .unwrap();

    // Only Alice (age 25) should have the relationship
    assert_eq!(graph.edge_count(), 1);
}

// バインディングキャッシュ + プロパティインデックス連携テスト (#74)

#[test]
fn test_bound_variable_reused_not_duplicated() {
    // MATCH で束縛した変数を CREATE で再利用するとノードが新規作成されないことを確認
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    // ノードが新規作成されていないこと
    assert_eq!(graph.node_count(), 2);
    // エッジが1本だけ作成されていること
    assert_eq!(graph.edge_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(0)); // created_nodes = 0
    assert_eq!(result.rows[0].columns[1], Value::Int(1)); // created_edges = 1
}

#[test]
fn test_same_variable_matched_twice_uses_cache() {
    // 同一変数を MATCH 内で2パターン参照した場合、2回目は既存バインドを流用
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice", age: 30})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (b:Person {name: "Alice", age: 25})"#, // name 同じ・age 違う
    )
    .unwrap();

    // name="Alice" かつ age=30 のノードだけがマッチするはず
    let result = execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (a:Person {age: 30}) RETURN a.name, a.age"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[1], Value::Int(30));
}

#[test]
fn test_index_accelerated_match_create() {
    // プロパティインデックスが CREATE KNOWS の MATCH フェーズを高速化することを確認
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    // インデックス作成
    execute_with(&mut executor, "CREATE INDEX ON :Person(name)").unwrap();

    // ノード作成（インデックスに自動登録される）
    execute_with(&mut executor, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute_with(&mut executor, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

    // インデックス経由で MATCH し、エッジを作成
    let result = execute_with(
        &mut executor,
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    // ノード再作成なし、エッジ1本
    assert_eq!(result.rows[0].columns[0], Value::Int(0)); // created_nodes
    assert_eq!(result.rows[0].columns[1], Value::Int(1)); // created_edges

    // インデックスが a・b を引けていることをプロパティで確認
    let check = execute_with(
        &mut executor,
        r#"MATCH (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"}) RETURN a.name, b.name"#,
    )
    .unwrap();
    assert_eq!(check.row_count(), 1);
}

// ========== MATCH + SET tests ==========

#[test]
fn test_match_set_property() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
}

#[test]
fn test_match_set_multiple_properties() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31, n.city = "Tokyo""#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
    assert_eq!(
        node.get_property("city"),
        Some(&PropertyValue::String("Tokyo".to_string()))
    );
}

#[test]
fn test_match_set_with_return() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n.name, n.age"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::Int(31));
}

#[test]
fn test_match_set_edge_property() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    execute(
        &mut graph,
        r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r.since = 2024"#,
    )
    .unwrap();

    let edge = graph.edges().next().unwrap();
    assert_eq!(edge.get_property("since"), Some(&PropertyValue::Int(2024)));
}

// ========== SET += and SET n:Label tests ==========

#[test]
fn test_set_merge_properties_adds_new_props() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n += {age: 31, city: "Tokyo"}"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    // Existing property 'name' must be preserved
    assert_eq!(
        node.get_property("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
    // 'age' should be updated to 31
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(31)));
    // New property 'city' should be added
    assert_eq!(
        node.get_property("city"),
        Some(&PropertyValue::String("Tokyo".to_string()))
    );
}

#[test]
fn test_set_merge_properties_with_return() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n += {age: 31, city: "Tokyo"} RETURN n.name, n.age, n.city"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::Int(31));
    assert_eq!(
        result.rows[0].columns[2],
        Value::String("Tokyo".to_string())
    );
}

#[test]
fn test_set_add_label_to_node() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age >= 20 SET n:Adult"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    // Node should now be matchable by both Person and Adult labels
    assert!(node.has_label("Person"));
    assert!(node.has_label("Adult"));
}

#[test]
fn test_set_add_label_idempotent() {
    // Adding the same label twice should not duplicate it
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    execute(&mut graph, r#"MATCH (n:Person) SET n:Adult"#).unwrap();
    execute(&mut graph, r#"MATCH (n:Person) SET n:Adult"#).unwrap();

    let node = graph.nodes().next().unwrap();
    let label_count = node.labels.iter().filter(|l| *l == "Adult").count();
    assert_eq!(label_count, 1, "Label 'Adult' should appear only once");
}

#[test]
fn test_set_add_label_match_by_new_label() {
    // After SET n:Adult, the node should be matchable by (n:Adult)
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age >= 20 SET n:Adult"#,
    )
    .unwrap();

    let result = execute(&mut graph, r#"MATCH (n:Adult) RETURN n.name"#).unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_set_add_label_with_return() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) SET n:Adult RETURN n.name, labels(n)"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    // labels(n) should contain both Person and Adult
    let labels_val = &result.rows[0].columns[1];
    if let Value::List(labels) = labels_val {
        assert!(labels.contains(&Value::String("Person".to_string())));
        assert!(labels.contains(&Value::String("Adult".to_string())));
    } else {
        panic!("Expected list of labels, got {:?}", labels_val);
    }
}

#[test]
fn test_set_merge_properties_edge() {
    // SET += on an edge should also merge properties
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    execute(
        &mut graph,
        r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r += {weight: 5}"#,
    )
    .unwrap();

    let edge = graph.edges().next().unwrap();
    // Existing property 'since' must be preserved
    assert_eq!(edge.get_property("since"), Some(&PropertyValue::Int(2020)));
    // New property 'weight' should be added
    assert_eq!(edge.get_property("weight"), Some(&PropertyValue::Int(5)));
}

// ========== MERGE tests ==========

#[test]
fn test_merge_create_new() {
    let mut graph = Graph::new();

    execute(&mut graph, r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("Person"));
    assert_eq!(
        node.get_property("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
}

#[test]
fn test_merge_match_existing() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    assert_eq!(graph.node_count(), 1);

    execute(&mut graph, r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

    // Should still be 1 node (matched existing)
    assert_eq!(graph.node_count(), 1);
}

#[test]
fn test_merge_on_create_set() {
    let mut graph = Graph::new();

    execute(
        &mut graph,
        r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25"#,
    )
    .unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(25)));
}

#[test]
fn test_merge_on_match_set() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 25})"#).unwrap();

    execute(
        &mut graph,
        r#"MERGE (n:Person {name: "Alice"}) ON MATCH SET n.age = 30"#,
    )
    .unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(30)));
}

#[test]
fn test_merge_with_match_prefix() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) MERGE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    assert_eq!(graph.edge_count(), 1);
}

#[test]
fn test_merge_with_return() {
    let mut graph = Graph::new();

    let result = execute(
        &mut graph,
        r#"MERGE (n:Person {name: "Charlie"}) ON CREATE SET n.age = 25 RETURN n.name, n.age"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Charlie".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::Int(25));
}

// ========== MATCH + REMOVE tests ==========

#[test]
fn test_match_remove_property() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Alice", age: 30, city: "Tokyo"})"#,
    )
    .unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), None);
    assert_eq!(
        node.get_property("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
}

#[test]
fn test_match_remove_multiple_properties() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Alice", age: 30, city: "Tokyo"})"#,
    )
    .unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age, n.city"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("age"), None);
    assert_eq!(node.get_property("city"), None);
    assert_eq!(
        node.get_property("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
}

#[test]
fn test_match_remove_with_return() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_match_remove_edge_property() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // First set a property on the edge
    execute(
        &mut graph,
        r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) SET r.since = 2024"#,
    )
    .unwrap();

    // Then remove it
    execute(
        &mut graph,
        r#"MATCH (a:Person)-[r:KNOWS]->(b:Person) REMOVE r.since"#,
    )
    .unwrap();

    let edge = graph.edges().next().unwrap();
    assert_eq!(edge.get_property("since"), None);
}

#[test]
fn test_match_remove_label() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) REMOVE n:Person"#,
    )
    .unwrap();

    let node = graph.nodes().next().unwrap();
    assert!(!node.has_label("Person"), "Person label should be removed");
}

// ========== UNWIND tests ==========

#[test]
fn test_unwind_list() {
    let mut graph = Graph::new();

    let result = execute(&mut graph, "UNWIND [1, 2, 3] AS x RETURN x").unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(result.rows[1].columns[0], Value::Int(2));
    assert_eq!(result.rows[2].columns[0], Value::Int(3));
}

#[test]
fn test_unwind_string_list() {
    let mut graph = Graph::new();

    let result = execute(
        &mut graph,
        r#"UNWIND ["Alice", "Bob", "Charlie"] AS name RETURN name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
    assert_eq!(
        result.rows[2].columns[0],
        Value::String("Charlie".to_string())
    );
}

#[test]
fn test_unwind_empty_list() {
    let mut graph = Graph::new();

    let result = execute(&mut graph, "UNWIND [] AS x RETURN x").unwrap();

    assert_eq!(result.row_count(), 0);
}

// ========== Parser tests for new features ==========

#[test]
fn test_parse_match_create() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (b:Person {name: "Bob"})"#).unwrap();

    // Verify the query parses and executes correctly
    let result = execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:FRIENDS]->(b)"#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_match_set() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n.age"#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_merge() {
    let mut graph = Graph::new();

    let result = execute(
        &mut graph,
        r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25 ON MATCH SET n.age = 30 RETURN n"#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_remove() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n.name"#,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_unwind() {
    let mut graph = Graph::new();

    let result = execute(&mut graph, "UNWIND [1, 2, 3] AS x RETURN x");
    assert!(result.is_ok());
}

// ========== Constraint DDL tests ==========
