//! プロパティ索引、件数の近道、UNWIND による一括作成、RETURN だけの文、ORDER BY / WITH / CREATE … RETURN の回帰テスト。

use super::*;

#[test]
fn test_create_index_and_lookup() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    // Create index
    let stmt = Parser::new("CREATE INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    // Create some nodes
    let stmt = Parser::new("CREATE (n:Person {name: 'Alice', age: 30})")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("CREATE (n:Person {name: 'Bob', age: 25})")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("CREATE (n:Person {name: 'Alice', age: 35})")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    // Re-create index to pick up existing nodes
    let stmt = Parser::new("DROP INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("CREATE INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    // Query using indexed property
    let stmt = Parser::new("MATCH (n:Person {name: 'Alice'}) RETURN n.name")
        .unwrap()
        .parse()
        .unwrap();
    let rs = executor.execute(stmt).unwrap();
    assert_eq!(rs.row_count(), 2);
}

#[test]
fn test_show_indexes() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    let stmt = Parser::new("CREATE INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("CREATE INDEX ON :Employee(email)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    let stmt = Parser::new("SHOW INDEXES").unwrap().parse().unwrap();
    let result = executor.execute(stmt).unwrap();
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_drop_index() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    let stmt = Parser::new("CREATE INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("DROP INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    let stmt = Parser::new("SHOW INDEXES").unwrap().parse().unwrap();
    let result = executor.execute(stmt).unwrap();
    assert_eq!(result.row_count(), 0);
}

#[test]
fn test_index_auto_update_on_create() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    // Create index first
    let stmt = Parser::new("CREATE INDEX ON :Person(name)")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    // Create nodes - they should be automatically indexed
    let stmt = Parser::new("CREATE (n:Person {name: 'Alice'})")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();
    let stmt = Parser::new("CREATE (n:Person {name: 'Bob'})")
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    // Verify index has the entries
    let alices = executor
        .property_index()
        .find_by_property("name", &PropertyValue::String("Alice".to_string()));
    assert_eq!(alices.len(), 1);
}

#[test]
fn test_count_node_optimization() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:Person {name: 'Alice'})").unwrap();
    execute(&mut graph, "CREATE (n:Person {name: 'Bob'})").unwrap();
    execute(&mut graph, "CREATE (n:Person {name: 'Carol'})").unwrap();
    let result = execute(&mut graph, "MATCH (n:Person) RETURN count(n)").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(3));
}

#[test]
fn test_group_by_count() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:Person {city: 'Tokyo'})").unwrap();
    execute(&mut graph, "CREATE (n:Person {city: 'Tokyo'})").unwrap();
    execute(&mut graph, "CREATE (n:Person {city: 'Osaka'})").unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n.city, count(n)").unwrap();
    assert_eq!(result.row_count(), 2); // Two distinct cities
    assert_eq!(result.columns, vec!["n.city", "count(n)"]);
}

#[test]
fn test_group_by_avg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:Person {dept: 'Eng', salary: 100})").unwrap();
    execute(&mut graph, "CREATE (n:Person {dept: 'Eng', salary: 200})").unwrap();
    execute(&mut graph, "CREATE (n:Person {dept: 'Sales', salary: 150})").unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n.dept, avg(n.salary)").unwrap();
    assert_eq!(result.row_count(), 2);

    // Find the Eng row and verify avg = 150.0
    let eng_row = result
        .rows
        .iter()
        .find(|r| r.columns[0] == Value::String("Eng".to_string()));
    assert!(eng_row.is_some());
    if let Some(row) = eng_row {
        assert_eq!(row.columns[1], Value::Float(150.0));
    }
}

#[test]
fn test_simple_count_star() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:A)").unwrap();
    execute(&mut graph, "CREATE (n:A)").unwrap();
    let result = execute(&mut graph, "MATCH (n:A) RETURN count(*)").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2));
}

// ========== UNWIND + CREATE batch write tests (task #70) ==========

#[test]
fn test_unwind_create_from_map_params() {
    // UNWIND $nodes AS n CREATE (:Person {id: n.id, name: n.name}) でノードを一括作成
    let mut graph = Graph::new();
    let stmt = Parser::new("UNWIND $nodes AS n CREATE (:Person {id: n.id, name: n.name})")
        .unwrap()
        .parse()
        .unwrap();

    let nodes_list = Value::List(vec![
        Value::Map({
            let mut m = HashMap::new();
            m.insert("id".to_string(), Value::Int(1));
            m.insert("name".to_string(), Value::String("Alice".to_string()));
            m
        }),
        Value::Map({
            let mut m = HashMap::new();
            m.insert("id".to_string(), Value::Int(2));
            m.insert("name".to_string(), Value::String("Bob".to_string()));
            m
        }),
        Value::Map({
            let mut m = HashMap::new();
            m.insert("id".to_string(), Value::Int(3));
            m.insert("name".to_string(), Value::String("Carol".to_string()));
            m
        }),
    ]);

    let mut params = HashMap::new();
    params.insert("nodes".to_string(), nodes_list);

    Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(graph.node_count(), 3);

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN n.id, n.name ORDER BY n.id",
    )
    .unwrap();
    assert_eq!(result.row_count(), 3);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(
        result.rows[0].columns[1],
        Value::String("Alice".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::Int(2));
    assert_eq!(result.rows[2].columns[0], Value::Int(3));
}

#[test]
fn test_unwind_create_bulk_1000_nodes() {
    // 1000件のノードを UNWIND+CREATE で一括作成できることを確認
    let mut graph = Graph::new();

    let items: Vec<Value> = (1..=1000)
        .map(|i| {
            Value::Map({
                let mut m = HashMap::new();
                m.insert("id".to_string(), Value::Int(i));
                m
            })
        })
        .collect();

    let stmt = Parser::new("UNWIND $items AS item CREATE (:Bulk {id: item.id})")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert("items".to_string(), Value::List(items));

    Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(graph.node_count(), 1000);
}

#[test]
fn test_unwind_create_scalar_list() {
    // スカラーリストの UNWIND+CREATE（n.prop アクセスなし）
    let mut graph = Graph::new();
    let stmt = Parser::new("UNWIND $vals AS v CREATE (:Tag {value: v})")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert(
        "vals".to_string(),
        Value::List(vec![
            Value::String("alpha".to_string()),
            Value::String("beta".to_string()),
        ]),
    );

    Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(graph.node_count(), 2);
}

#[test]
fn test_unwind_map_property_access_null_for_missing_key() {
    // マップに存在しないキーへのアクセスは Null を返す（エラーにならない）
    let mut graph = Graph::new();
    let stmt = Parser::new("UNWIND $nodes AS n CREATE (:X {a: n.a, b: n.b})")
        .unwrap()
        .parse()
        .unwrap();

    let mut params = HashMap::new();
    params.insert(
        "nodes".to_string(),
        Value::List(vec![Value::Map({
            let mut m = HashMap::new();
            m.insert("a".to_string(), Value::Int(1));
            // "b" key is absent
            m
        })]),
    );

    Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();

    assert_eq!(graph.node_count(), 1);
    let node = graph.nodes().next().unwrap();
    assert_eq!(node.get_property("a"), Some(&PropertyValue::Int(1)));
    // missing key evaluates to Null; Null properties may be stored as PropertyValue::Null
    let b = node.get_property("b");
    assert!(b.is_none() || b == Some(&PropertyValue::Null));
}

#[test]
fn test_unwind_inline_json_map_create() {
    // benchmark.py の bench_unwind_batch_create が生成するクエリ形式 (Task 88)
    // json.dumps で {"key": value} 形式になるインラインマップリテラル + CREATE
    let mut graph = Graph::new();
    let result = execute(
        &mut graph,
        r#"UNWIND [{"id": 0, "name": "Alice0", "city": "Tokyo"}, {"id": 1, "name": "Bob1", "city": "Osaka"}] AS item CREATE (:UnwindBench {id: item.id, name: item.name, city: item.city})"#,
    );
    assert!(
        result.is_ok(),
        "UNWIND with inline JSON map literal failed: {:?}",
        result
    );
    assert_eq!(graph.node_count(), 2);

    let result = execute(
        &mut graph,
        "MATCH (n:UnwindBench) RETURN n.id, n.name, n.city ORDER BY n.id",
    )
    .unwrap();
    assert_eq!(result.row_count(), 2);
    assert_eq!(result.rows[0].columns[0], Value::Int(0));
    assert_eq!(
        result.rows[0].columns[1],
        Value::String("Alice0".to_string())
    );
    assert_eq!(
        result.rows[0].columns[2],
        Value::String("Tokyo".to_string())
    );
    assert_eq!(result.rows[1].columns[0], Value::Int(1));
    assert_eq!(result.rows[1].columns[1], Value::String("Bob1".to_string()));
    assert_eq!(
        result.rows[1].columns[2],
        Value::String("Osaka".to_string())
    );
}

// ========== Standalone RETURN tests (Task 89) ==========

#[test]
fn test_execute_standalone_return_literal() {
    let mut graph = Graph::new();
    let stmt = Parser::new("RETURN 1 + 1 AS result")
        .unwrap()
        .parse()
        .unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    assert_eq!(result.columns, vec!["result"]);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2));
}

#[test]
fn test_execute_standalone_return_string() {
    let mut graph = Graph::new();
    let stmt = Parser::new(r#"RETURN 'hello' AS greeting"#)
        .unwrap()
        .parse()
        .unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    assert_eq!(result.columns, vec!["greeting"]);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello".to_string())
    );
}

#[test]
fn test_execute_standalone_return_multiple() {
    let mut graph = Graph::new();
    let stmt = Parser::new("RETURN 1 AS a, 2 AS b")
        .unwrap()
        .parse()
        .unwrap();
    let result = Executor::new(&mut graph).execute(stmt).unwrap();
    assert_eq!(result.columns, vec!["a", "b"]);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(result.rows[0].columns[1], Value::Int(2));
}

// ---- bug/118: silently ignored clauses ----

#[test]
fn set_then_remove_in_one_statement_applies_both() {
    let mut graph = Graph::new();
    seed_people(&mut graph);
    execute(
        &mut graph,
        "MATCH (n:P {name: 'a'}) SET n.age = 10 REMOVE n.name SET n:Q",
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (n:Q) RETURN n.age, n.name").unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].columns, vec![Value::Int(10), Value::Null]);

    execute(&mut graph, "MATCH (n:Q) REMOVE n:Q SET n.name = 'z'").unwrap();
    assert_eq!(
        execute(&mut graph, "MATCH (n:Q) RETURN n")
            .unwrap()
            .rows
            .len(),
        0
    );
    let r = execute(&mut graph, "MATCH (n:P) WHERE n.age = 10 RETURN n.name").unwrap();
    assert_eq!(first_column(&r), strings(&["z"]));
}

#[test]
fn order_by_key_not_in_return_is_applied() {
    let mut graph = Graph::new();
    seed_people(&mut graph);
    let r = execute(&mut graph, "MATCH (n:P) RETURN n.name ORDER BY n.age").unwrap();
    assert_eq!(first_column(&r), strings(&["a", "b", "c"]));
    let r = execute(
        &mut graph,
        "MATCH (n:P) RETURN n.name ORDER BY n.age DESC LIMIT 2",
    )
    .unwrap();
    assert_eq!(first_column(&r), strings(&["c", "b"]));
}

#[test]
fn order_by_function_expression() {
    let mut graph = Graph::new();
    for name in ["b", "C", "a"] {
        execute(&mut graph, &format!("CREATE (:P {{name: '{name}'}})")).unwrap();
    }
    let r = execute(
        &mut graph,
        "MATCH (n:P) RETURN n.name ORDER BY toUpper(n.name)",
    )
    .unwrap();
    assert_eq!(first_column(&r), strings(&["a", "b", "C"]));
}

#[test]
fn order_by_non_returned_key_with_aggregation_is_an_error() {
    let mut graph = Graph::new();
    seed_people(&mut graph);
    let err = execute(
        &mut graph,
        "MATCH (n:P) RETURN n.name, count(*) ORDER BY n.age",
    );
    assert!(err.is_err(), "must not silently skip sorting: {err:?}");
    // Referring to a returned column (or alias) works.
    let r = execute(
        &mut graph,
        "MATCH (n:P) RETURN n.name AS nm, count(*) AS c ORDER BY nm DESC",
    )
    .unwrap();
    assert_eq!(first_column(&r), strings(&["c", "b", "a"]));
}

#[test]
fn with_order_by_is_applied_without_aggregation() {
    let mut graph = Graph::new();
    seed_people(&mut graph);
    let r = execute(
        &mut graph,
        "MATCH (n:P) WITH n ORDER BY n.age DESC RETURN n.name",
    )
    .unwrap();
    assert_eq!(first_column(&r), strings(&["c", "b", "a"]));
    let r = execute(
        &mut graph,
        "MATCH (n:P) WITH n ORDER BY n.age LIMIT 2 RETURN n.name",
    )
    .unwrap();
    assert_eq!(first_column(&r), strings(&["a", "b"]));
}

#[test]
fn with_distinct_removes_duplicates() {
    let mut graph = Graph::new();
    for _ in 0..20 {
        execute(&mut graph, "CREATE (:D {g: 1, h: 2, i: 3})").unwrap();
    }
    // Several columns: HashMap iteration order differs between binding maps,
    // so a Debug-string key used to treat identical rows as distinct.
    let r = execute(
        &mut graph,
        "MATCH (n:D) WITH DISTINCT n.g AS g, n.h AS h, n.i AS i RETURN g, h, i",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
}

#[test]
fn create_return_projects_created_elements() {
    let mut graph = Graph::new();
    let r = execute(
        &mut graph,
        "CREATE (a:P {name: 'a'})-[r:R {w: 2}]->(b:P {name: 'b'}) RETURN a.name, r.w, b.name",
    )
    .unwrap();
    assert_eq!(r.columns, vec!["a.name", "r.w", "b.name"]);
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::String("a".into()),
            Value::Int(2),
            Value::String("b".into())
        ]
    );
}

#[test]
fn create_reuses_variable_bound_earlier_in_same_clause() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (a:A), (a)-[:R]->(b:B)").unwrap();
    assert_eq!(graph.node_count(), 2, "`a` must not be created twice");
    assert_eq!(graph.edge_count(), 1);
}

#[test]
fn match_create_return_and_unwind_create_return() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:P {name: 'a'})").unwrap();
    let r = execute(
        &mut graph,
        "MATCH (a:P {name: 'a'}) CREATE (a)-[r:R]->(b:Q {n: 1}) RETURN b.n",
    )
    .unwrap();
    assert_eq!(first_column(&r), vec![Value::Int(1)]);

    let r = execute(
        &mut graph,
        "UNWIND [1, 2, 3] AS x CREATE (n:U {v: x}) SET n.w = x RETURN n.w ORDER BY n.w DESC",
    )
    .unwrap();
    assert_eq!(
        first_column(&r),
        vec![Value::Int(3), Value::Int(2), Value::Int(1)]
    );
}
