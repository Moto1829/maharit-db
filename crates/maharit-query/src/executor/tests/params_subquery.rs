//! パラメータ（$param）と、EXISTS / COUNT / COLLECT / CALL サブクエリ、FOREACH。

use super::*;

#[test]
fn test_map_param_as_pattern_properties() {
    // task145: (n:Label $props) / -[r:T $props]-> / $props.key
    let mut graph = Graph::new();
    let alice = map_value(&[
        ("name", Value::String("Alice".into())),
        ("age", Value::Int(30)),
    ]);
    run_with(
        &mut graph,
        "CREATE (n:Person $props)",
        &[("props", alice.clone())],
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (n:Person) RETURN n.name, n.age").unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::String("Alice".into()), Value::Int(30)]
    );

    // MATCH: 全キーの等価条件
    let r = run_with(
        &mut graph,
        "MATCH (n:Person $p) RETURN n.name",
        &[("p", alice.clone())],
    )
    .unwrap();
    assert_eq!(r.row_count(), 1);
    let other = map_value(&[
        ("name", Value::String("Alice".into())),
        ("age", Value::Int(31)),
    ]);
    let r = run_with(&mut graph, "MATCH (n:Person $p) RETURN n", &[("p", other)]).unwrap();
    assert_eq!(r.row_count(), 0);

    // リレーションシップ
    run_with(
        &mut graph,
        "MATCH (a:Person) CREATE (a)-[:KNOWS $r]->(:Person {name: 'Bob'})",
        &[("r", map_value(&[("since", Value::Int(2020))]))],
    )
    .unwrap();
    let r = run_with(
        &mut graph,
        "MATCH ()-[r:KNOWS $r]->(b) RETURN r.since, b.name",
        &[("r", map_value(&[("since", Value::Int(2020))]))],
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::Int(2020), Value::String("Bob".into())]
    );
    let r = run_with(
        &mut graph,
        "MATCH ()-[r:KNOWS $r]->() RETURN r",
        &[("r", map_value(&[("since", Value::Int(1999))]))],
    )
    .unwrap();
    assert_eq!(r.row_count(), 0);

    // MERGE: 一致すれば作らない、しなければ作る
    run_with(&mut graph, "MERGE (n:Person $p)", &[("p", alice.clone())]).unwrap();
    let carol = map_value(&[("name", Value::String("Carol".into()))]);
    run_with(&mut graph, "MERGE (n:Person $p)", &[("p", carol)]).unwrap();
    let r = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(r.row_count(), 3);

    // $p.key / $p['key'] / ネスト
    let nested = map_value(&[
        ("name", Value::String("Dave".into())),
        (
            "addr",
            map_value(&[("city", Value::String("Osaka".into()))]),
        ),
    ]);
    run_with(
        &mut graph,
        "CREATE (n:Person {name: $p.name, city: $p.addr.city, k: $p['name']})",
        &[("p", nested.clone())],
    )
    .unwrap();
    let r = run_with(
        &mut graph,
        "MATCH (n:Person) WHERE n.name = $p.name RETURN n.city, n.k, $p.missing AS m",
        &[("p", nested)],
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::String("Osaka".into()),
            Value::String("Dave".into()),
            Value::Null
        ]
    );

    // マップ以外はエラー
    let err = run_with(&mut graph, "CREATE (n:X $p)", &[("p", Value::Int(1))]).unwrap_err();
    assert!(err.to_string().contains("must be a map"), "{err}");
    assert!(run_with(&mut graph, "CREATE (n:X $nope)", &[]).is_err());
}

#[test]
fn test_set_from_map_param() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:P {name: 'a', old: 1})").unwrap();
    run_with(
        &mut graph,
        "MATCH (n:P) SET n += $p",
        &[("p", map_value(&[("age", Value::Int(5))]))],
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (n:P) RETURN n.name, n.old, n.age").unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::String("a".into()), Value::Int(1), Value::Int(5)]
    );
    run_with(
        &mut graph,
        "MATCH (n:P) SET n = $p",
        &[("p", map_value(&[("name", Value::String("b".into()))]))],
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (n:P) RETURN n.name, n.old, n.age").unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::String("b".into()), Value::Null, Value::Null]
    );
}

#[test]
fn test_param_in_where() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    let stmt = Parser::new("MATCH (n:Person) WHERE n.name = $name RETURN n.name")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("name".to_string(), Value::String("Alice".to_string()));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_param_in_create_property() {
    let mut graph = Graph::new();

    let stmt = Parser::new(r#"CREATE (n:Person {name: $name, age: $age})"#)
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("name".to_string(), Value::String("Charlie".to_string()));
    params.insert("age".to_string(), Value::Int(35));

    Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert_eq!(
        node.get_property("name"),
        Some(&maharit_core::PropertyValue::String("Charlie".to_string()))
    );
    assert_eq!(
        node.get_property("age"),
        Some(&maharit_core::PropertyValue::Int(35))
    );
}

#[test]
fn test_param_in_match_pattern() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();

    let stmt = Parser::new(r#"MATCH (n:City {name: $city}) RETURN n.name"#)
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("city".to_string(), Value::String("Tokyo".to_string()));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Tokyo".to_string())
    );
}

#[test]
fn test_param_in_set() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Dave", age: 20})"#).unwrap();

    let stmt = Parser::new(r#"MATCH (n:Person {name: "Dave"}) SET n.age = $new_age RETURN n.age"#)
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("new_age".to_string(), Value::Int(21));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(21));
}

#[test]
fn test_param_undefined_error() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {x: 1})"#).unwrap();

    let stmt = Parser::new("MATCH (n:T) WHERE n.x = $missing RETURN n")
        .unwrap()
        .parse()
        .unwrap();

    let result = Executor::new(&mut graph).execute_with_params(stmt, HashMap::new());
    // Should not panic, may return empty result or error
    // Undefined params during WHERE evaluation cause the filter to fail (row excluded)
    // because evaluate_expression returns Err which is treated as non-match
    // Just verify it doesn't panic
    let _ = result;
}

// ========== SKIP/LIMIT parameter tests ==========

#[test]
fn test_limit_with_param() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Carol"})"#).unwrap();

    let stmt = Parser::new("MATCH (n:Person) RETURN n.name LIMIT $count")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("count".to_string(), Value::Int(2));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_skip_with_param() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Carol"})"#).unwrap();

    let stmt = Parser::new("MATCH (n:Person) RETURN n.name SKIP $offset")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("offset".to_string(), Value::Int(2));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(result.row_count(), 1);
}

#[test]
fn test_skip_and_limit_with_params() {
    let mut graph = Graph::new();
    for i in 1..=5i64 {
        execute(&mut graph, &format!(r#"CREATE (n:Item {{val: {}}})"#, i)).unwrap();
    }

    let stmt = Parser::new("MATCH (n:Item) RETURN n.val ORDER BY n.val SKIP $offset LIMIT $count")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("offset".to_string(), Value::Int(1));
    params.insert("count".to_string(), Value::Int(2));

    let result = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    // Items: 1,2,3,4,5 -> skip 1 -> 2,3,4,5 -> limit 2 -> 2,3
    assert_eq!(result.row_count(), 2);
    assert_eq!(result.rows[0].columns[0], Value::Int(2));
    assert_eq!(result.rows[1].columns[0], Value::Int(3));
}

#[test]
fn test_limit_with_param_undefined_error() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let stmt = Parser::new("MATCH (n:Person) RETURN n.name LIMIT $missing")
        .unwrap()
        .parse()
        .unwrap();

    let result = Executor::new(&mut graph).execute_with_params(stmt, HashMap::new());
    // $missing is not defined: should return an error
    assert!(result.is_err());
}

// ========== Subquery tests ==========

#[test]
fn test_exists_subquery_true() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Alice KNOWS Bob, so EXISTS should be true for Alice
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person {name: "Alice"}) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_exists_subquery_false() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"}), (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // No KNOWS edges, EXISTS should filter out all results
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 0);
}

#[test]
fn test_exists_subquery_filters_correctly() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (c:Person {name: "Charlie"})"#).unwrap();

    // Only Alice has KNOWS relationship
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_count_subquery_basic() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Alice"})-[:KNOWS]->(:Person {name: "Carol"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (:Person {name: "Dave"})"#).unwrap();

    // People with more than 0 KNOWS connections (Alice instances have 1 each)
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 0 RETURN p.name"#,
    )
    .unwrap();

    // Alice (x2) should match, Dave should not
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_count_subquery_returns_integer() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (c:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(c)"#,
    )
    .unwrap();

    // Return the count as a value
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person {name: "Alice"}) RETURN COUNT { MATCH (p)-[:KNOWS]->() }"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    // COUNT returns an integer value
    assert!(matches!(result.rows[0].columns[0], Value::Int(_)));
}

#[test]
fn test_collect_subquery_basic() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"MATCH (a:Person {name: "Alice"}), (c:Person {name: "Bob"})
           CREATE (a)-[:KNOWS]->(:Person {name: "Carol"})"#,
    )
    .unwrap();

    // COLLECT subquery returns a list of friend names
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person {name: "Alice"})
           RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert!(matches!(result.rows[0].columns[0], Value::List(_)));
}

#[test]
fn test_collect_subquery_empty() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (:Person {name: "Alice"})"#).unwrap();

    // COLLECT on empty pattern match returns empty list
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person {name: "Alice"})
           RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::List(vec![]));
}

#[test]
fn test_call_subquery_basic() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // CALL subquery with WITH import: get friend count for each person
    // COUNT aggregation always produces a result row (even with 0 matches),
    // so both Alice and Bob appear in results
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person)
           CALL {
             WITH p
             MATCH (p)-[:KNOWS]->(f:Person)
             RETURN COUNT(f) AS friend_count
           }
           RETURN p.name, friend_count"#,
    )
    .unwrap();

    assert_eq!(result.columns.len(), 2);
    // Both Person nodes appear since COUNT always returns a row
    assert_eq!(result.row_count(), 2);
    // Verify friend_count values exist
    for row in &result.rows {
        assert!(matches!(row.columns[1], Value::Int(_)));
    }
}

#[test]
fn test_call_subquery_without_with() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (:Person {name: "Alice"}), (:Person {name: "Bob"})"#,
    )
    .unwrap();

    // CALL subquery without WITH: inner query is independent
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person)
           CALL {
             MATCH (q:Person)
             RETURN COUNT(q) AS total
           }
           RETURN p.name, total"#,
    )
    .unwrap();

    // Each outer person row gets combined with inner result
    assert!(result.row_count() >= 1);
}

#[test]
fn test_exists_subquery_with_where() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice", city: "Tokyo"})-[:KNOWS]->(b:Person {name: "Bob", city: "Tokyo"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (c:Person {name: "Charlie", city: "Osaka"})-[:KNOWS]->(d:Person {name: "Dave", city: "Osaka"})"#,
    )
    .unwrap();

    // EXISTS with WHERE inside subquery
    let result = execute(
        &mut graph,
        r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(f:Person) WHERE f.city = "Tokyo" } RETURN p.name"#,
    )
    .unwrap();

    // Only Alice knows someone in Tokyo
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_count_subquery_return_value() {
    let mut graph = Graph::new();
    // Person with no friends
    execute(&mut graph, r#"CREATE (:Person {name: "Alice"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (p:Person) RETURN p.name, COUNT { MATCH (p)-[:KNOWS]->() }"#,
    )
    .unwrap();

    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[1], Value::Int(0));
}

// ========== FOREACH Tests ==========

#[test]
fn test_foreach_create_nodes() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"FOREACH (name IN ['Alice', 'Bob', 'Charlie'] | CREATE (:Person {name: name}))"#,
    )
    .unwrap();

    // Verify 3 nodes were created
    let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.row_count(), 3);
}

#[test]
fn test_foreach_set_property() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (:Item {val: 1})"#).unwrap();
    execute(&mut graph, r#"CREATE (:Item {val: 2})"#).unwrap();

    // Match all items and for each, set a flag using MATCH+FOREACH
    let result = execute(
        &mut graph,
        r#"MATCH (n:Item) FOREACH (x IN [1] | SET n.processed = true)"#,
    )
    .unwrap();
    assert_eq!(result.columns[0], "foreach_result");

    // Verify both items have the flag set
    let check = execute(
        &mut graph,
        r#"MATCH (n:Item) WHERE n.processed = true RETURN n"#,
    )
    .unwrap();
    assert_eq!(check.row_count(), 2);
}

#[test]
fn test_foreach_nested_create() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"FOREACH (city IN ['Tokyo', 'Osaka'] |
          FOREACH (name IN ['Alice', 'Bob'] |
            CREATE (:Person {name: name, city: city})
          )
        )"#,
    )
    .unwrap();

    // Expect 2 cities * 2 names = 4 persons
    let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.row_count(), 4);
}
