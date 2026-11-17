//! MATCH・集約・ORDER BY / SKIP / LIMIT・最短経路・OPTIONAL MATCH・CASE・WITH・UNION など読み取りの基本。
//! WHERE の押し下げと Value の JSON 変換もここ。

use super::*;

#[test]
fn test_where_equality_pushdown_basic() {
    let mut graph = Graph::new();
    seed_cities(&mut graph);
    let rs = execute(
        &mut graph,
        "MATCH (n:Person) WHERE n.city = 'Tokyo' RETURN n.name",
    )
    .unwrap();
    // Tokyo の 2 件のみ（Osaka と city 欠損は除外）
    assert_eq!(rs.rows.len(), 2);
}

#[test]
fn test_where_or_is_not_overpruned() {
    let mut graph = Graph::new();
    seed_cities(&mut graph);
    // OR は押し下げ対象外。両都市が返ることを確認（AND として誤処理されない）。
    let rs = execute(
        &mut graph,
        "MATCH (n:Person) WHERE n.city = 'Tokyo' OR n.city = 'Osaka' RETURN n.name",
    )
    .unwrap();
    assert_eq!(rs.rows.len(), 3);
}

#[test]
fn test_where_equality_and_range() {
    let mut graph = Graph::new();
    seed_cities(&mut graph);
    // city = 'Tokyo' AND age > 30 → 'a'(40) のみ
    let rs = execute(
        &mut graph,
        "MATCH (n:Person) WHERE n.city = 'Tokyo' AND n.age > 30 RETURN n.name",
    )
    .unwrap();
    assert_eq!(rs.rows.len(), 1);
}

#[test]
fn test_property_index_updated_on_set_and_delete() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    execute_with(&mut executor, "CREATE (:P {age: 10})").unwrap();
    execute_with(&mut executor, "CREATE INDEX ON :P(age)").unwrap();

    // Index-backed lookup finds the node at its original value.
    let rs = execute_with(&mut executor, "MATCH (n:P {age: 10}) RETURN n").unwrap();
    assert_eq!(rs.rows.len(), 1);

    // SET must update the index so the new value is found and the old is not.
    execute_with(&mut executor, "MATCH (n:P {age: 10}) SET n.age = 20").unwrap();
    let rs = execute_with(&mut executor, "MATCH (n:P {age: 20}) RETURN n").unwrap();
    assert_eq!(
        rs.rows.len(),
        1,
        "index not updated on SET (index-backed lookup missed the node)"
    );
    let rs = execute_with(&mut executor, "MATCH (n:P {age: 10}) RETURN n").unwrap();
    assert_eq!(rs.rows.len(), 0);

    // DELETE must remove the node from the index (no stale hit).
    execute_with(&mut executor, "MATCH (n:P {age: 20}) DELETE n").unwrap();
    let rs = execute_with(&mut executor, "MATCH (n:P {age: 20}) RETURN n").unwrap();
    assert_eq!(rs.rows.len(), 0);
}

#[test]
fn test_range_pushdown_with_index() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    for age in [10, 20, 30, 40, 50] {
        execute_with(&mut executor, &format!("CREATE (:P {{age: {}}})", age)).unwrap();
    }
    execute_with(&mut executor, "CREATE INDEX ON :P(age)").unwrap();

    let n = |ex: &mut Executor<'_>, q: &str| execute_with(ex, q).unwrap().rows.len();
    assert_eq!(n(&mut executor, "MATCH (n:P) WHERE n.age > 30 RETURN n"), 2); // 40,50
    assert_eq!(
        n(&mut executor, "MATCH (n:P) WHERE n.age >= 30 RETURN n"),
        3
    ); // 30,40,50
    assert_eq!(n(&mut executor, "MATCH (n:P) WHERE n.age < 30 RETURN n"), 2); // 10,20
    assert_eq!(
        n(&mut executor, "MATCH (n:P) WHERE n.age <= 20 RETURN n"),
        2
    ); // 10,20
    // Literal on the left (operator must be flipped).
    assert_eq!(n(&mut executor, "MATCH (n:P) WHERE 30 < n.age RETURN n"), 2); // 40,50

    // After SET the index must stay correct (maintenance + range pushdown).
    execute_with(
        &mut executor,
        "MATCH (n:P) WHERE n.age = 10 SET n.age = 100",
    )
    .unwrap();
    assert_eq!(n(&mut executor, "MATCH (n:P) WHERE n.age > 60 RETURN n"), 1); // only 100
}

#[test]
fn test_range_pushdown_mixed_int_float() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    execute_with(&mut executor, "CREATE INDEX ON :M(v)").unwrap();
    execute_with(&mut executor, "CREATE (:M {v: 10})").unwrap();
    execute_with(&mut executor, "CREATE (:M {v: 25.5})").unwrap();
    execute_with(&mut executor, "CREATE (:M {v: 40})").unwrap();
    // v > 20 must include the float 25.5 (int/float range indexes are unioned).
    let rs = execute_with(&mut executor, "MATCH (n:M) WHERE n.v > 20 RETURN n").unwrap();
    assert_eq!(rs.rows.len(), 2); // 25.5, 40
}

#[test]
fn test_count_shortcut_and_filtered_counts() {
    let mut graph = Graph::new();
    seed_cities(&mut graph); // 4 Person (3 with city: 2 Tokyo/1 Osaka, 1 without)

    // Short-circuit path: count over label = all Person.
    assert_eq!(
        count_of(&execute(&mut graph, "MATCH (n:Person) RETURN count(*)").unwrap()),
        4
    );
    assert_eq!(
        count_of(&execute(&mut graph, "MATCH (n:Person) RETURN count(n)").unwrap()),
        4
    );
    // No label → all nodes.
    assert_eq!(
        count_of(&execute(&mut graph, "MATCH (n) RETURN count(*)").unwrap()),
        4
    );
    // AS alias still short-circuits.
    assert_eq!(
        count_of(&execute(&mut graph, "MATCH (n:Person) RETURN count(*) AS c").unwrap()),
        4
    );

    // WHERE must still filter (shortcut must NOT fire): only Tokyo = 2.
    assert_eq!(
        count_of(
            &execute(
                &mut graph,
                "MATCH (n:Person) WHERE n.city = 'Tokyo' RETURN count(*)"
            )
            .unwrap()
        ),
        2
    );
    // Inline property must still filter: Osaka = 1.
    assert_eq!(
        count_of(
            &execute(
                &mut graph,
                "MATCH (n:Person {city: 'Osaka'}) RETURN count(*)"
            )
            .unwrap()
        ),
        1
    );
}

#[test]
fn test_where_equality_matches_full_scan_semantics() {
    // 押し下げの有無で結果が変わらないことを、同義な等価述語と
    // インラインプロパティの件数一致で確認する。
    let mut graph = Graph::new();
    seed_cities(&mut graph);
    let via_where = execute(
        &mut graph,
        "MATCH (n:Person) WHERE n.city = 'Osaka' RETURN n.name",
    )
    .unwrap();
    let via_inline = execute(&mut graph, "MATCH (n:Person {city: 'Osaka'}) RETURN n.name").unwrap();
    assert_eq!(via_where.rows.len(), via_inline.rows.len());
    assert_eq!(via_where.rows.len(), 1);
}

// ────────────────────────────────────────────────────────────────
// Value::to_json (型情報を保ったまま JSON 表現に変換)
// ────────────────────────────────────────────────────────────────

#[test]
fn value_to_json_preserves_primitive_types() {
    assert_eq!(Value::Null.to_json(), serde_json::Value::Null);
    assert_eq!(Value::Bool(true).to_json(), serde_json::Value::Bool(true));
    assert_eq!(
        Value::Int(30).to_json(),
        serde_json::Value::Number(30.into())
    );
    assert_eq!(
        Value::String("alice".to_string()).to_json(),
        serde_json::Value::String("alice".to_string())
    );
}

#[test]
fn value_to_json_float_preserved() {
    let v = Value::Float(3.14).to_json();
    assert_eq!(v.as_f64(), Some(3.14));
}

#[test]
fn value_to_json_list_is_array() {
    let v = Value::List(vec![
        Value::Int(1),
        Value::String("two".to_string()),
        Value::Null,
    ])
    .to_json();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0], serde_json::json!(1));
    assert_eq!(arr[1], serde_json::Value::String("two".to_string()));
    assert_eq!(arr[2], serde_json::Value::Null);
}

#[test]
fn value_to_json_map_is_object() {
    let mut map = std::collections::HashMap::new();
    map.insert("name".to_string(), Value::String("Alice".to_string()));
    map.insert("age".to_string(), Value::Int(30));
    let v = Value::Map(map).to_json();
    let obj = v.as_object().unwrap();
    assert_eq!(obj.get("name"), Some(&serde_json::json!("Alice")));
    assert_eq!(obj.get("age"), Some(&serde_json::json!(30)));
}

#[test]
fn value_to_string_unquoted_skips_outer_quotes() {
    assert_eq!(
        Value::String("alice".to_string()).to_string_unquoted(),
        "alice"
    );
    assert_eq!(Value::Int(30).to_string_unquoted(), "30");
    assert_eq!(Value::Null.to_string_unquoted(), "null");
}

#[test]
fn test_create_node() {
    let mut graph = Graph::new();
    let result = execute(&mut graph, "CREATE (n:Person)").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(graph.node_count(), 1);

    let node = graph.nodes().next().unwrap();
    assert!(node.has_label("Person"));
}

#[test]
fn test_create_node_with_properties() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let node = graph.nodes().next().unwrap();
    assert_eq!(
        node.get_property("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
    assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(30)));
}

#[test]
fn test_create_path() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (a:Person)-[:KNOWS]->(b:Person)").unwrap();

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.edge_count(), 1);

    let edge = graph.edges().next().unwrap();
    assert_eq!(edge.label, "KNOWS");
}

#[test]
fn test_match_all_nodes() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_match_with_where() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) WHERE n.age > 28 RETURN n").unwrap();

    assert_eq!(result.row_count(), 1);
}

#[test]
fn test_match_return_property() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_match_path() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
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
fn test_count_star() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN COUNT(*)").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(3));
}

#[test]
fn test_count_variable() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN COUNT(n)").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2));
}

#[test]
fn test_sum() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN SUM(n.age)").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(90));
}

#[test]
fn test_avg() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN AVG(n.age)").unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Float(30.0));
}

#[test]
fn test_min_max() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let min_result = execute(&mut graph, "MATCH (n:Person) RETURN MIN(n.age)").unwrap();
    let max_result = execute(&mut graph, "MATCH (n:Person) RETURN MAX(n.age)").unwrap();

    assert_eq!(min_result.rows[0].columns[0], Value::Int(25));
    assert_eq!(max_result.rows[0].columns[0], Value::Int(35));
}

#[test]
fn test_variable_length_path() {
    let mut graph = Graph::new();
    // Create a chain: Alice -> Bob -> Charlie -> David
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Get Alice and Bob IDs
    let _alice_id = graph
        .nodes()
        .find(|n| n.properties.get("name") == Some(&PropertyValue::String("Alice".to_string())))
        .unwrap()
        .id;
    let bob_id = graph
        .nodes()
        .find(|n| n.properties.get("name") == Some(&PropertyValue::String("Bob".to_string())))
        .unwrap()
        .id;

    // Create Charlie and David
    let charlie_id = graph.create_node("Person");
    graph
        .get_node_mut(charlie_id)
        .unwrap()
        .set_property("name", PropertyValue::String("Charlie".to_string()));
    graph.create_edge(bob_id, charlie_id, "KNOWS").unwrap();

    let david_id = graph.create_node("Person");
    graph
        .get_node_mut(david_id)
        .unwrap()
        .set_property("name", PropertyValue::String("David".to_string()));
    graph.create_edge(charlie_id, david_id, "KNOWS").unwrap();

    // Test: Find all people reachable from Alice in 2 hops
    let result = execute(
        &mut graph,
        "MATCH (a:Person {name: \"Alice\"})-[:KNOWS*2]->(b:Person) RETURN b.name",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Charlie".to_string())
    );

    // Test: Find all people reachable from Alice in 1 to 3 hops
    let result = execute(
        &mut graph,
        "MATCH (a:Person {name: \"Alice\"})-[:KNOWS*1..3]->(b:Person) RETURN b.name",
    )
    .unwrap();
    assert_eq!(result.row_count(), 3); // Bob, Charlie, David
}

#[test]
fn test_variable_length_path_range() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // *2..3 should find C (2 hops) but not B (1 hop)
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[:NEXT*2..3]->(b:Node) RETURN b.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::String("C".to_string()));
}

// ========== Path function tests ==========

#[test]
fn test_path_length_function() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // Test length() function - path from A to C has 2 edges
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[r:NEXT*1..2]->(b:Node) RETURN b.name, length(r)"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 2);

    // Find the row for C (2 hops)
    let c_row = result
        .rows
        .iter()
        .find(|row| row.columns[0] == Value::String("C".to_string()))
        .expect("Should find C");
    assert_eq!(c_row.columns[1], Value::Int(2));

    // Find the row for B (1 hop)
    let b_row = result
        .rows
        .iter()
        .find(|row| row.columns[0] == Value::String("B".to_string()))
        .expect("Should find B");
    assert_eq!(b_row.columns[1], Value::Int(1));
}

#[test]
fn test_path_nodes_function() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // Test nodes() function - returns list of nodes in path
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[r:NEXT*2]->(b:Node) RETURN nodes(r)"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);

    // Should be a list with 3 nodes (A, B, C)
    if let Value::List(nodes) = &result.rows[0].columns[0] {
        assert_eq!(nodes.len(), 3);
    } else {
        panic!("Expected List value");
    }
}

#[test]
fn test_path_relationships_function() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // Test relationships() function - returns list of edges in path
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[r:NEXT*2]->(b:Node) RETURN relationships(r)"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);

    // Should be a list with 2 edges
    if let Value::List(edges) = &result.rows[0].columns[0] {
        assert_eq!(edges.len(), 2);
    } else {
        panic!("Expected List value");
    }
}

#[test]
fn test_variable_length_path_star_only() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // [*] means 1..unlimited, should find B and C (1 and 2 hops)
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[:NEXT*]->(b:Node) RETURN b.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 2); // B (1 hop) and C (2 hops)
}

#[test]
fn test_variable_length_path_zero_hop() {
    let mut graph = Graph::new();
    // Create: A -> B -> C
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    // [*0..2] means 0, 1, or 2 hops
    // 0 hops: A itself (if it matches the target pattern)
    // 1 hop: B
    // 2 hops: C
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"})-[:NEXT*0..2]->(b:Node) RETURN b.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 3); // A (0 hops), B (1 hop), C (2 hops)
}

// ========== ORDER BY tests ==========

#[test]
fn test_order_by_asc() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string()));
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[2].columns[0],
        Value::String("Charlie".to_string())
    );
}

#[test]
fn test_order_by_desc() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Charlie".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string()));
}

#[test]
fn test_order_by_string() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name ORDER BY n.name ASC",
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

// ========== LIMIT tests ==========

#[test]
fn test_limit() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name LIMIT 2").unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_limit_larger_than_result() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n.name LIMIT 10").unwrap();

    assert_eq!(result.row_count(), 2);
}

// ========== SKIP tests ==========

#[test]
fn test_skip() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1",
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    );
}

#[test]
fn test_skip_and_limit() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "David", age: 40})"#).unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age SKIP 1 LIMIT 2",
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    );
}

// ========== DISTINCT tests ==========

#[test]
fn test_distinct() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#,
    )
    .unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN DISTINCT n.city").unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_distinct_with_order_by() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Alice", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Bob", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", city: "Osaka"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN DISTINCT n.city ORDER BY n.city",
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Osaka".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Tokyo".to_string())
    );
}

// ========== NULLS FIRST/LAST tests ==========

#[test]
fn test_nulls_last_default_asc() {
    let mut graph = Graph::new();
    // Create nodes with and without age property
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    // ASC default: NULLS LAST
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Charlie".to_string())
    ); // age 25
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Alice".to_string())
    ); // age 30
    assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string())); // NULL
    assert_eq!(result.rows[2].columns[1], Value::Null);
}

#[test]
fn test_nulls_first_default_desc() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    // DESC default: NULLS FIRST
    assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string())); // NULL
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Alice".to_string())
    ); // age 30
    assert_eq!(
        result.rows[2].columns[0],
        Value::String("Charlie".to_string())
    ); // age 25
}

#[test]
fn test_nulls_first_explicit() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age ASC NULLS FIRST",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    // NULLS FIRST explicitly
    assert_eq!(result.rows[0].columns[0], Value::String("Bob".to_string())); // NULL
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    ); // age 25
    assert_eq!(
        result.rows[2].columns[0],
        Value::String("Alice".to_string())
    ); // age 30
}

#[test]
fn test_nulls_last_explicit() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap(); // no age
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 25})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC NULLS LAST",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    // NULLS LAST explicitly with DESC
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    ); // age 30
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    ); // age 25
    assert_eq!(result.rows[2].columns[0], Value::String("Bob".to_string())); // NULL
}

// ========== TopN optimization tests ==========

#[test]
fn test_topn_optimization() {
    let mut graph = Graph::new();
    // Create 10 nodes
    for i in 1..=10 {
        execute(
            &mut graph,
            &format!(
                r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#,
                i,
                i * 10
            ),
        )
        .unwrap();
    }

    // Request only top 3 by age DESC
    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC LIMIT 3",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(result.rows[0].columns[1], Value::Int(100)); // age 100
    assert_eq!(result.rows[1].columns[1], Value::Int(90)); // age 90
    assert_eq!(result.rows[2].columns[1], Value::Int(80)); // age 80
}

/// `ORDER BY … LIMIT` は残る行だけを組み立てる。全件を並べてから切った結果と
/// 同じになること（同じキーの行の順序、SKIP、別名や返さない値での並べ替えを含む）。
#[test]
fn test_topn_matches_full_sort() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "UNWIND range(1, 40) AS i CREATE (:T {id: i, grp: i % 4, name: 'n' + toString(i % 7)})",
    )
    .unwrap();
    let rows = |graph: &mut Graph, q: &str| -> Vec<String> {
        execute(graph, q)
            .unwrap()
            .rows
            .iter()
            .map(|r| format!("{:?}", r.columns))
            .collect()
    };
    for (ret, order) in [
        ("n.id", "n.grp"),
        ("n.id", "n.grp DESC, n.name"),
        ("n.id AS id, n.grp AS g", "g DESC"),
        ("n.name, n.id", "n.name DESC"),
        ("n.id + 1 AS next", "n.grp, next DESC"),
    ] {
        let base = format!("MATCH (n:T) RETURN {ret} ORDER BY {order}");
        let all = rows(&mut graph, &base);
        assert_eq!(all.len(), 40);
        for (skip, limit) in [(0, 1), (0, 5), (3, 5), (38, 5), (50, 5), (0, 0), (0, 39)] {
            let got = rows(&mut graph, &format!("{base} SKIP {skip} LIMIT {limit}"));
            let want: Vec<String> = all.iter().skip(skip).take(limit).cloned().collect();
            assert_eq!(got, want, "{base} SKIP {skip} LIMIT {limit}");
        }
        let got = rows(&mut graph, &format!("{base} LIMIT 7"));
        assert_eq!(got, all[..7], "{base} LIMIT 7");
    }
}

#[test]
fn test_topn_with_skip() {
    let mut graph = Graph::new();
    // Create 10 nodes
    for i in 1..=10 {
        execute(
            &mut graph,
            &format!(
                r#"CREATE (n:Person {{name: "Person{}", age: {}}})"#,
                i,
                i * 10
            ),
        )
        .unwrap();
    }

    // Skip 2, take 3 (should get 3rd, 4th, 5th highest)
    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC SKIP 2 LIMIT 3",
    )
    .unwrap();

    assert_eq!(result.row_count(), 3);
    assert_eq!(result.rows[0].columns[1], Value::Int(80)); // 3rd highest
    assert_eq!(result.rows[1].columns[1], Value::Int(70)); // 4th highest
    assert_eq!(result.rows[2].columns[1], Value::Int(60)); // 5th highest
}

// ========== shortestPath / allShortestPaths tests ==========

#[test]
fn test_shortest_path_function() {
    let mut graph = Graph::new();
    // Create: A -> B -> C -> D
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    let d = graph.create_node("Node");
    graph
        .get_node_mut(d)
        .unwrap()
        .set_property("name", PropertyValue::String("D".to_string()));
    graph.create_edge(c, d, "NEXT").unwrap();

    // Find shortest path from A to D
    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"}), (d:Node {name: "D"}) RETURN shortestPath(a, d)"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    if let Value::Path { nodes, edges } = &result.rows[0].columns[0] {
        assert_eq!(nodes.len(), 4); // A, B, C, D
        assert_eq!(edges.len(), 3); // 3 edges
        assert_eq!(nodes[0], a);
        assert_eq!(nodes[3], d);
    } else {
        panic!("Expected Path value, got {:?}", result.rows[0].columns[0]);
    }
}

#[test]
fn test_shortest_path_no_path() {
    let mut graph = Graph::new();
    // Create disconnected nodes: A, B (no edge between them)
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));

    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"}), (b:Node {name: "B"}) RETURN shortestPath(a, b)"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Null);
}

#[test]
fn test_all_shortest_paths_single() {
    let mut graph = Graph::new();
    // Create: A -> B -> C (only one path)
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));
    graph.create_edge(a, b, "NEXT").unwrap();

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));
    graph.create_edge(b, c, "NEXT").unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"}), (c:Node {name: "C"}) RETURN allShortestPaths(a, c)"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    if let Value::List(paths) = &result.rows[0].columns[0] {
        assert_eq!(paths.len(), 1);
        if let Value::Path { nodes, edges } = &paths[0] {
            assert_eq!(nodes.len(), 3); // A, B, C
            assert_eq!(edges.len(), 2);
        } else {
            panic!("Expected Path value in list");
        }
    } else {
        panic!("Expected List value");
    }
}

#[test]
fn test_all_shortest_paths_multiple() {
    let mut graph = Graph::new();
    // Create diamond: A -> B -> D, A -> C -> D
    let a = graph.create_node("Node");
    graph
        .get_node_mut(a)
        .unwrap()
        .set_property("name", PropertyValue::String("A".to_string()));

    let b = graph.create_node("Node");
    graph
        .get_node_mut(b)
        .unwrap()
        .set_property("name", PropertyValue::String("B".to_string()));

    let c = graph.create_node("Node");
    graph
        .get_node_mut(c)
        .unwrap()
        .set_property("name", PropertyValue::String("C".to_string()));

    let d = graph.create_node("Node");
    graph
        .get_node_mut(d)
        .unwrap()
        .set_property("name", PropertyValue::String("D".to_string()));

    graph.create_edge(a, b, "E1").unwrap();
    graph.create_edge(a, c, "E2").unwrap();
    graph.create_edge(b, d, "E3").unwrap();
    graph.create_edge(c, d, "E4").unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Node {name: "A"}), (d:Node {name: "D"}) RETURN allShortestPaths(a, d)"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    if let Value::List(paths) = &result.rows[0].columns[0] {
        assert_eq!(paths.len(), 2); // Two shortest paths: A-B-D and A-C-D
        for path in paths {
            if let Value::Path { nodes, edges } = path {
                assert_eq!(nodes.len(), 3); // A, middle, D
                assert_eq!(edges.len(), 2);
                assert_eq!(nodes[0], a);
                assert_eq!(nodes[2], d);
            } else {
                panic!("Expected Path value in list");
            }
        }
    } else {
        panic!("Expected List value");
    }
}

// ========== OPTIONAL MATCH tests ==========

#[test]
fn test_optional_match_with_match() {
    let mut graph = Graph::new();
    // Create Alice -> Bob, Charlie (no outgoing edge)
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

    // OPTIONAL MATCH: Alice has a friend, Charlie doesn't
    let result = execute(
        &mut graph,
        r#"MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name ORDER BY a.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 3); // Alice, Bob, Charlie
    // Alice -> Bob
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
    // Bob has no outgoing KNOWS
    assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
    assert_eq!(result.rows[1].columns[1], Value::Null);
    // Charlie has no outgoing KNOWS
    assert_eq!(
        result.rows[2].columns[0],
        Value::String("Charlie".to_string())
    );
    assert_eq!(result.rows[2].columns[1], Value::Null);
}

#[test]
fn test_optional_match_no_match() {
    let mut graph = Graph::new();
    // Create just Alice with no relationships
    execute(&mut graph, r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::Null);
}

#[test]
fn test_optional_match_all_match() {
    let mut graph = Graph::new();
    // Everyone knows someone
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}) OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) RETURN a.name, b.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::String("Bob".to_string()));
}

// ========== CASE WHEN tests ==========

#[test]
fn test_case_when_searched_in_where() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 15})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 65})"#,
    )
    .unwrap();

    // Use CASE in WHERE to filter: only adults (age >= 18)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE CASE WHEN n.age >= 18 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    );
}

#[test]
fn test_case_when_with_else() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 15})"#).unwrap();

    // CASE with multiple WHEN and ELSE
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE CASE WHEN n.age < 18 THEN false WHEN n.age >= 18 THEN true ELSE false END RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_case_when_no_match_returns_null() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    // CASE without ELSE and no match returns NULL
    // This test uses the fact that NULL is not true, so WHERE filters it out
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE CASE WHEN n.age < 18 THEN true END RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 0); // Alice's age is 30, CASE returns NULL, not true
}

#[test]
fn test_case_when_simple_form() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Alice", status: 1})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", status: 2})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", status: 1})"#,
    )
    .unwrap();

    // Simple CASE: CASE n.status WHEN 1 THEN true ELSE false END
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE CASE n.status WHEN 1 THEN true ELSE false END RETURN n.name ORDER BY n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    );
}

// ========== WITH clause tests ==========

#[test]
fn test_with_passthrough() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    // WITH n simply passes bindings through
    let result = execute(
        &mut graph,
        "MATCH (n:Person) WITH n RETURN n.name ORDER BY n.name",
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
}

#[test]
fn test_with_alias() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    // WITH n.name AS name projects property into a new variable
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WITH n.name AS name RETURN name ORDER BY name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::String("Bob".to_string()));
}

#[test]
fn test_with_limit() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Charlie"})"#).unwrap();

    // WITH n LIMIT 2 restricts intermediate results
    let result = execute(&mut graph, "MATCH (n:Person) WITH n LIMIT 2 RETURN n.name").unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_with_where_filter() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    // WITH + WHERE filters on projected values
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WITH n WHERE n.age > 28 RETURN n.name ORDER BY n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Charlie".to_string())
    );
}

// ========== WITH group aggregation tests (task_95) ==========

#[test]
fn test_with_group_count() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Alice", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Charlie", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Bob", city: "Osaka"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WITH n.city AS city, COUNT(n) AS cnt RETURN city, cnt ORDER BY cnt DESC"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2, "Expected 2 groups (Tokyo, Osaka)");
    // First row: Tokyo with cnt=2
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Tokyo".to_string())
    );
    assert_eq!(result.rows[0].columns[1], Value::Int(2));
    // Second row: Osaka with cnt=1
    assert_eq!(
        result.rows[1].columns[0],
        Value::String("Osaka".to_string())
    );
    assert_eq!(result.rows[1].columns[1], Value::Int(1));
}

#[test]
fn test_with_group_sum() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (:Sale {region: "East", amount: 100})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (:Sale {region: "East", amount: 200})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (:Sale {region: "West", amount: 50})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (o:Sale) WITH o.region AS region, SUM(o.amount) AS total RETURN region, total ORDER BY total DESC"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(result.rows[0].columns[0], Value::String("East".to_string()));
    assert_eq!(result.rows[0].columns[1], Value::Int(300));
    assert_eq!(result.rows[1].columns[0], Value::String("West".to_string()));
    assert_eq!(result.rows[1].columns[1], Value::Int(50));
}

#[test]
fn test_with_aggregate_pipeline_then_return() {
    // WITH aggregation の結果をさらに RETURN で絞り込む
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (:Item {cat: "A", val: 10})"#).unwrap();
    execute(&mut graph, r#"CREATE (:Item {cat: "A", val: 20})"#).unwrap();
    execute(&mut graph, r#"CREATE (:Item {cat: "B", val: 5})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (i:Item) WITH i.cat AS cat, COUNT(i) AS cnt RETURN cat, cnt ORDER BY cat"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(result.rows[0].columns[0], Value::String("A".to_string()));
    assert_eq!(result.rows[0].columns[1], Value::Int(2));
    assert_eq!(result.rows[1].columns[0], Value::String("B".to_string()));
    assert_eq!(result.rows[1].columns[1], Value::Int(1));
}

// ========== Regex match (=~) tests ==========

#[test]
fn test_regex_match() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Anna"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name =~ "A.*" RETURN n.name ORDER BY n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::String("Anna".to_string()));
}

#[test]
fn test_regex_no_match() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name =~ "B.*" RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 0);
}

#[test]
fn test_regex_full_match() {
    // =~ should be a full match, not partial
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    // "lic" should NOT match "Alice" (partial match)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name =~ "lic" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 0);

    // ".*lic.*" should match "Alice"
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name =~ ".*lic.*" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
}

#[test]
fn test_regex_type_mismatch_returns_no_rows() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    // =~ on non-string operand evaluates to false in WHERE context
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age =~ ".*" RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 0);
}

// ========== UNION / UNION ALL tests ==========

#[test]
fn test_union_basic() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Company {name: "Acme"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
    assert_eq!(result.columns, vec!["n.name".to_string()]);
}

#[test]
fn test_union_dedup() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Company {name: "Alice"})"#).unwrap();

    // UNION should remove duplicates
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_union_all() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Company {name: "Alice"})"#).unwrap();

    // UNION ALL should keep duplicates
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_union_column_count_mismatch() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Company {name: "Acme"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) RETURN n.name, n.age UNION MATCH (n:Company) RETURN n.name"#,
    );

    assert!(result.is_err());
}

// ========== MATCH + CREATE tests ==========
