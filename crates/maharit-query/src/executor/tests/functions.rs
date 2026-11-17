//! 文字列・数値・リストのスカラー関数と、統計系の集約関数。

use super::*;

#[test]
fn test_starts_with_operator() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name STARTS WITH "Ali" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_ends_with_operator() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name ENDS WITH "ice" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("Alice".to_string())
    );
}

#[test]
fn test_starts_with_case_insensitive() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name STARTS WITH "ali" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_ends_with_case_insensitive() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.name ENDS WITH "ICE" RETURN n.name"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_is_normalized() {
    let mut graph = Graph::new();
    // NFC normalized string (precomposed)
    execute(&mut graph, "CREATE (n:Text {value: \"\u{00e9}\"})").unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Text) WHERE n.value IS NORMALIZED RETURN n.value"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_is_not_normalized() {
    let mut graph = Graph::new();
    // NFD string (decomposed: e + combining acute accent)
    execute(&mut graph, "CREATE (n:Text {value: \"e\u{0301}\"})").unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Text) WHERE n.value IS NORMALIZED RETURN n.value"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_string_operators_with_non_string() {
    // WHERE clause errors are treated as non-matching (unwrap_or(false)),
    // so type mismatches result in 0 rows rather than errors.
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {age: 30})"#).unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age STARTS WITH "3" RETURN n"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 0);

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age ENDS WITH "0" RETURN n"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 0);

    let result = execute(
        &mut graph,
        r#"MATCH (n:Person) WHERE n.age IS NORMALIZED RETURN n"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 0);
}

// ========== String function tests ==========

#[test]
fn test_trim_functions() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "  hello  "})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN trim(n.val)"#).unwrap();
    assert_eq!(result.columns, vec!["trim(n.val)"]);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello".to_string())
    );

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN ltrim(n.val)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello  ".to_string())
    );

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN rtrim(n.val)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("  hello".to_string())
    );
}

#[test]
fn test_case_conversion() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "Hello World"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toLower(n.val)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello world".to_string())
    );

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toUpper(n.val)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("HELLO WORLD".to_string())
    );
}

#[test]
fn test_reverse() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "abcde"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN reverse(n.val)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("edcba".to_string())
    );
}

#[test]
fn test_substring() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

    // 2引数: start から末尾まで
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN substring(n.val, 6)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("world".to_string())
    );

    // 3引数: start から len 文字
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN substring(n.val, 0, 5)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello".to_string())
    );
}

#[test]
fn test_left_right() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN left(n.val, 5)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello".to_string())
    );

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN right(n.val, 5)"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("world".to_string())
    );
}

#[test]
fn test_split() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "a,b,c"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN split(n.val, ",")"#).unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::List(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ])
    );
}

#[test]
fn test_replace() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "hello world"})"#).unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:T) RETURN replace(n.val, "world", "rust")"#,
    )
    .unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("hello rust".to_string())
    );
}

#[test]
fn test_to_string() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:T {i: 42, f: 3.14, b: true, s: "hi"})"#,
    )
    .unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.i)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::String("42".to_string()));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.f)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::String("3.14".to_string()));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.b)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::String("true".to_string()));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toString(n.s)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::String("hi".to_string()));
}

#[test]
fn test_size() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: "hello"})"#).unwrap();

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN size(n.val)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));
}

#[test]
fn test_abs() {
    let mut graph = Graph::new();
    // Test negative int and float
    execute(&mut graph, "CREATE (n:T)").unwrap();
    graph
        .get_node_mut(0)
        .unwrap()
        .set_property("i", PropertyValue::Int(-5));
    graph
        .get_node_mut(0)
        .unwrap()
        .set_property("f", PropertyValue::Float(-3.14));

    let result = execute(&mut graph, "MATCH (n:T) RETURN abs(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));

    let result = execute(&mut graph, "MATCH (n:T) RETURN abs(n.f)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(3.14));

    // Positive values stay the same
    let mut graph2 = Graph::new();
    execute(&mut graph2, "CREATE (n:T {i: 5, f: 3.14})").unwrap();
    let result = execute(&mut graph2, "MATCH (n:T) RETURN abs(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));
}

#[test]
fn test_ceil_floor() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {f: 3.2, i: 5})").unwrap();
    graph
        .get_node_mut(0)
        .unwrap()
        .set_property("g", PropertyValue::Float(-2.7));

    let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.f)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(4));

    let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.f)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(3));

    let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.g)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(-2));

    let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.g)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(-3));

    // Int passthrough
    let result = execute(&mut graph, "MATCH (n:T) RETURN ceil(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));

    let result = execute(&mut graph, "MATCH (n:T) RETURN floor(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));
}

#[test]
fn test_round() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {f: 3.567, i: 5})").unwrap();

    // 1 argument: round to integer
    let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(4));

    // Int passthrough
    let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(5));

    // 2 arguments: round to precision
    let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f, 2)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(3.57));

    let result = execute(&mut graph, "MATCH (n:T) RETURN round(n.f, 1)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(3.6));
}

#[test]
fn test_sign() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {pos: 5, zero: 0, fpos: 2.5})").unwrap();
    graph
        .get_node_mut(0)
        .unwrap()
        .set_property("neg", PropertyValue::Int(-3));
    graph
        .get_node_mut(0)
        .unwrap()
        .set_property("fneg", PropertyValue::Float(-1.5));

    let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.pos)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(1));

    let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.neg)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(-1));

    let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.zero)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(0));

    let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.fpos)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(1));

    let result = execute(&mut graph, "MATCH (n:T) RETURN sign(n.fneg)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(-1));
}

#[test]
fn test_rand() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

    let result = execute(&mut graph, "MATCH (n:T) RETURN rand()").unwrap();
    match &result.rows[0].columns[0] {
        Value::Float(f) => {
            assert!(
                *f >= 0.0 && *f < 1.0,
                "rand() should be in [0.0, 1.0), got {}",
                f
            );
        }
        other => panic!("Expected Float, got {:?}", other),
    }
}

#[test]
fn test_isnan() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {f: 1.0, i: 5})").unwrap();

    // Normal float is not NaN
    let result = execute(&mut graph, "MATCH (n:T) RETURN isNaN(n.f)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    // Int is never NaN
    let result = execute(&mut graph, "MATCH (n:T) RETURN isNaN(n.i)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));
}

#[test]
fn test_log_log10_sqrt() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (n:T {val: 100, fval: 2.718281828459045})",
    )
    .unwrap();

    // log(e) ≈ 1.0
    let result = execute(&mut graph, "MATCH (n:T) RETURN log(n.fval)").unwrap();
    match &result.rows[0].columns[0] {
        Value::Float(f) => assert!((f - 1.0).abs() < 0.0001),
        other => panic!("Expected Float, got {:?}", other),
    }

    // log10(100) = 2.0
    let result = execute(&mut graph, "MATCH (n:T) RETURN log10(n.val)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(2.0));

    // sqrt(100) = 10.0
    let result = execute(&mut graph, "MATCH (n:T) RETURN sqrt(n.val)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(10.0));
}

#[test]
fn test_e_pi() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

    let result = execute(&mut graph, "MATCH (n:T) RETURN e()").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(std::f64::consts::E));

    let result = execute(&mut graph, "MATCH (n:T) RETURN pi()").unwrap();
    assert_eq!(
        result.rows[0].columns[0],
        Value::Float(std::f64::consts::PI)
    );
}

// ========== Metadata function tests ==========

#[test]
fn test_id() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[r:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Node ID
    let result = execute(&mut graph, "MATCH (n:Person) RETURN id(n) ORDER BY id(n)").unwrap();
    assert_eq!(result.rows.len(), 2);
    // IDs should be non-negative integers
    match &result.rows[0].columns[0] {
        Value::Int(id) => assert!(*id >= 0),
        other => panic!("expected Int, got {:?}", other),
    }

    // Edge ID
    let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN id(r)").unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0].columns[0] {
        Value::Int(id) => assert!(*id >= 0),
        other => panic!("expected Int, got {:?}", other),
    }
}

#[test]
fn test_element_id() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[r:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Node element ID
    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN elementId(n) ORDER BY id(n)",
    )
    .unwrap();
    match &result.rows[0].columns[0] {
        Value::String(s) => assert!(s.starts_with("node:")),
        other => panic!("expected String starting with 'node:', got {:?}", other),
    }

    // Edge element ID
    let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN elementId(r)").unwrap();
    match &result.rows[0].columns[0] {
        Value::String(s) => assert!(s.starts_with("edge:")),
        other => panic!("expected String starting with 'edge:', got {:?}", other),
    }
}

#[test]
fn test_type_function() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (a:Person)-[:KNOWS]->(b:Person)"#).unwrap();

    let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN type(r)").unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("KNOWS".to_string())
    );
}

#[test]
fn test_start_end_node() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // startNode
    let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN startNode(r)").unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0].columns[0] {
        Value::NodeData {
            labels, properties, ..
        } => {
            assert!(labels.contains(&"Person".to_string()));
            assert_eq!(
                properties.get("name"),
                Some(&PropertyValue::String("Alice".to_string()))
            );
        }
        other => panic!("expected NodeData, got {:?}", other),
    }

    // endNode
    let result = execute(&mut graph, "MATCH (a)-[r:KNOWS]->(b) RETURN endNode(r)").unwrap();
    match &result.rows[0].columns[0] {
        Value::NodeData {
            labels, properties, ..
        } => {
            assert!(labels.contains(&"Person".to_string()));
            assert_eq!(
                properties.get("name"),
                Some(&PropertyValue::String("Bob".to_string()))
            );
        }
        other => panic!("expected NodeData, got {:?}", other),
    }
}

#[test]
fn test_labels() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN labels(n)").unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::List(vec![Value::String("Person".to_string())])
    );
}

#[test]
fn test_properties_keys() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    // properties - returns list of [key, value] pairs
    let result = execute(&mut graph, "MATCH (n:Person) RETURN properties(n)").unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0].columns[0] {
        Value::List(props) => {
            assert_eq!(props.len(), 2);
        }
        other => panic!("expected List, got {:?}", other),
    }

    // keys - returns list of key strings
    let result = execute(&mut graph, "MATCH (n:Person) RETURN keys(n)").unwrap();
    match &result.rows[0].columns[0] {
        Value::List(keys) => {
            assert_eq!(keys.len(), 2);
            // All should be strings
            for k in keys {
                match k {
                    Value::String(_) => {}
                    other => panic!("expected String key, got {:?}", other),
                }
            }
        }
        other => panic!("expected List, got {:?}", other),
    }
}

// ========== NULL handling function tests ==========

#[test]
fn test_coalesce() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: 42})"#).unwrap();

    // coalesce with non-null first value
    let result = execute(&mut graph, "MATCH (n:T) RETURN coalesce(n.val, 0)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(42));

    // coalesce with null first value - n.missing is null
    let result = execute(&mut graph, "MATCH (n:T) RETURN coalesce(n.missing, 99)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(99));

    // coalesce with all nulls
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN coalesce(n.missing, n.also_missing)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Null);
}

#[test]
fn test_nullif() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:T {val: 42})"#).unwrap();

    // Equal values -> NULL
    let result = execute(&mut graph, "MATCH (n:T) RETURN nullIf(n.val, 42)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Null);

    // Different values -> first value
    let result = execute(&mut graph, "MATCH (n:T) RETURN nullIf(n.val, 99)").unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(42));
}

// ========== Type conversion function tests ==========

#[test]
fn test_type_conversions() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:T {s_true: "true", s_false: "false", s_int: "42", s_float: "3.14", i: 10, f: 2.5, b: true})"#,
    )
    .unwrap();

    // toBoolean
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.s_true)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.s_false)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(false));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toBoolean(n.b)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Bool(true));

    // toFloat
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toFloat(n.i)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(10.0));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toFloat(n.s_float)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Float(3.14));

    // toInteger
    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toInteger(n.f)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(2));

    let result = execute(&mut graph, r#"MATCH (n:T) RETURN toInteger(n.s_int)"#).unwrap();
    assert_eq!(result.rows[0].columns[0], Value::Int(42));
}

// ========== Utility function tests ==========

#[test]
fn test_timestamp() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

    let result = execute(&mut graph, "MATCH (n:T) RETURN timestamp()").unwrap();
    match &result.rows[0].columns[0] {
        Value::Int(ts) => {
            // Should be a positive integer (Unix millis)
            assert!(*ts > 0);
            // Should be after year 2020 (1577836800000 millis)
            assert!(*ts > 1_577_836_800_000);
        }
        other => panic!("expected Int, got {:?}", other),
    }
}

#[test]
fn test_random_uuid() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {val: 1})").unwrap();

    let result = execute(&mut graph, "MATCH (n:T) RETURN randomUUID()").unwrap();
    match &result.rows[0].columns[0] {
        Value::String(uuid) => {
            // UUID format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
            assert_eq!(uuid.len(), 36);
            assert_eq!(&uuid[8..9], "-");
            assert_eq!(&uuid[13..14], "-");
            assert_eq!(&uuid[14..15], "4"); // version 4
            assert_eq!(&uuid[18..19], "-");
            assert_eq!(&uuid[23..24], "-");
        }
        other => panic!("expected String UUID, got {:?}", other),
    }
}

// ========== Task 41: List Operations ==========

#[test]
fn test_in_operator() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Item {val: 2})"#).unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:Item) WHERE n.val IN [1, 2, 3] RETURN n.val",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2));

    let result2 = execute(
        &mut graph,
        "MATCH (n:Item) WHERE n.val IN [5, 6, 7] RETURN n.val",
    )
    .unwrap();
    assert_eq!(result2.row_count(), 0);
}

#[test]
fn test_in_operator_null() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T {x: 1})").unwrap();
    let r = execute(&mut graph, "MATCH (n:T) WHERE 1 IN [1, 2] RETURN n.x").unwrap();
    assert_eq!(r.row_count(), 1);
}

#[test]
fn test_list_concatenation() {
    let mut graph = Graph::new();
    let result = execute(&mut graph, "UNWIND [1, 2] + [3, 4] AS x RETURN x").unwrap();
    assert_eq!(result.row_count(), 4);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(result.rows[3].columns[0], Value::Int(4));
}

#[test]
fn test_in_operator_multiple_nodes() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (a:Item {val: 200})").unwrap();
    execute(&mut graph, "CREATE (b:Item {val: 300})").unwrap();
    execute(&mut graph, "CREATE (c:Item {val: 999})").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:Item) WHERE n.val IN [200, 300] RETURN n.val",
    )
    .unwrap();
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_size_list() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN size([1, 2, 3])").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(3));
}

#[test]
fn test_reverse_list() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN reverse([1, 2, 3])").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn test_head_last_tail() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let r1 = execute(&mut graph, "MATCH (n:T) RETURN head([1, 2, 3])").unwrap();
    assert_eq!(r1.rows[0].columns[0], Value::Int(1));
    let r2 = execute(&mut graph, "MATCH (n:T) RETURN last([1, 2, 3])").unwrap();
    assert_eq!(r2.rows[0].columns[0], Value::Int(3));
    let r3 = execute(&mut graph, "MATCH (n:T) RETURN tail([1, 2, 3])").unwrap();
    assert_eq!(
        r3.rows[0].columns[0],
        Value::List(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn test_range() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let r1 = execute(&mut graph, "MATCH (n:T) RETURN range(1, 5)").unwrap();
    assert_eq!(
        r1.rows[0].columns[0],
        Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
            Value::Int(5)
        ])
    );
    let r2 = execute(&mut graph, "MATCH (n:T) RETURN range(1, 9, 2)").unwrap();
    assert_eq!(
        r2.rows[0].columns[0],
        Value::List(vec![
            Value::Int(1),
            Value::Int(3),
            Value::Int(5),
            Value::Int(7),
            Value::Int(9)
        ])
    );
}

#[test]
fn test_range_rejects_oversized_allocation() {
    // A range spanning the whole i64 domain must be rejected before
    // attempting to allocate ~9e18 elements (memory-exhaustion DoS).
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let err = execute(
        &mut graph,
        "MATCH (n:T) RETURN range(0, 9000000000000000000)",
    )
    .unwrap_err();
    assert!(
        matches!(err, ExecuteError::TypeError(ref m) if m.contains("exceeding the limit")),
        "expected range limit error, got {err:?}"
    );
}

#[test]
fn test_range_no_overflow_infinite_loop() {
    // Stepping past i64::MAX must terminate instead of wrapping around and
    // looping forever (CPU/memory-exhaustion DoS). The element count here
    // is tiny, so it is well under MAX_RANGE_LEN and must succeed.
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let r = execute(
        &mut graph,
        "MATCH (n:T) RETURN range(9223372036854775806, 9223372036854775807, 2)",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns[0],
        Value::List(vec![Value::Int(9223372036854775806)])
    );
}

#[test]
fn test_range_at_limit_boundary() {
    // A descending range must also be bounded correctly.
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let err = execute(
        &mut graph,
        "MATCH (n:T) RETURN range(9000000000000000000, 0, -1)",
    )
    .unwrap_err();
    assert!(matches!(err, ExecuteError::TypeError(_)));
}

#[test]
fn test_reduce() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN reduce(acc = 0, x IN [1, 2, 3, 4, 5] | acc + x)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(15));
}

#[test]
fn test_list_comprehension() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN [x IN [1, 2, 3, 4, 5] WHERE x > 2 | x * 2]",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::List(vec![Value::Int(6), Value::Int(8), Value::Int(10)])
    );
}

// ========== Task 42: Extended Aggregation Functions ==========

#[test]
fn test_percentile_cont() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {age: 10})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {age: 20})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {age: 40})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {age: 50})"#).unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person) RETURN percentileCont(n.age, 0.5)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    // Median of [10,20,30,40,50] with linear interpolation = 30.0
    assert_eq!(result.rows[0].columns[0], Value::Float(30.0));
}

#[test]
fn test_percentile_disc() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person2 {age: 10})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person2 {age: 20})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person2 {age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person2 {age: 40})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person2 {age: 50})"#).unwrap();

    let result = execute(
        &mut graph,
        "MATCH (n:Person2) RETURN percentileDisc(n.age, 0.5)",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    // Discrete median: ceil(0.5 * 5) - 1 = ceil(2.5) - 1 = 3 - 1 = 2 → value at index 2 = 30
    assert_eq!(result.rows[0].columns[0], Value::Int(30));
}

#[test]
fn test_stdev() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Score {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 5})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 5})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 7})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score {v: 9})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Score) RETURN stDev(n.v)").unwrap();
    assert_eq!(result.row_count(), 1);
    // Sample std dev of [2,4,4,4,5,5,7,9] = sqrt(32/7) ≈ 2.138
    if let Value::Float(v) = result.rows[0].columns[0] {
        assert!((v - 2.138).abs() < 0.01, "expected ~2.138, got {}", v);
    } else {
        panic!("expected Float, got {:?}", result.rows[0].columns[0]);
    }
}

#[test]
fn test_stdevp() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 4})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 5})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 5})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 7})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Score2 {v: 9})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Score2) RETURN stDevP(n.v)").unwrap();
    assert_eq!(result.row_count(), 1);
    // Population std dev of [2,4,4,4,5,5,7,9] = sqrt(32/8) = 2.0
    if let Value::Float(v) = result.rows[0].columns[0] {
        assert!((v - 2.0).abs() < 0.01, "expected ~2.0, got {}", v);
    } else {
        panic!("expected Float, got {:?}", result.rows[0].columns[0]);
    }
}

#[test]
fn test_count_distinct() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:City {name: "Tokyo"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:City {name: "Kyoto"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:City {name: "Osaka"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:City) RETURN COUNT(DISTINCT n.name)").unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(3));
}

#[test]
fn test_sum_distinct() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Val {v: 1})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val {v: 3})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Val) RETURN SUM(DISTINCT n.v)").unwrap();
    assert_eq!(result.row_count(), 1);
    // Sum of distinct values: 1 + 2 + 3 = 6
    assert_eq!(result.rows[0].columns[0], Value::Int(6));
}

#[test]
fn test_avg_distinct() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Val2 {v: 1})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val2 {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val2 {v: 2})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Val2 {v: 3})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Val2) RETURN AVG(DISTINCT n.v)").unwrap();
    assert_eq!(result.row_count(), 1);
    // Avg of distinct values: (1+2+3)/3 = 2.0
    assert_eq!(result.rows[0].columns[0], Value::Float(2.0));
}

#[test]
fn test_collect_distinct() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Tag {name: "rust"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Tag {name: "python"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Tag {name: "rust"})"#).unwrap();

    let result = execute(&mut graph, "MATCH (n:Tag) RETURN COLLECT(DISTINCT n.name)").unwrap();
    assert_eq!(result.row_count(), 1);
    // Should be a list with 2 distinct values
    if let Value::List(items) = &result.rows[0].columns[0] {
        assert_eq!(items.len(), 2);
    } else {
        panic!("expected List, got {:?}", result.rows[0].columns[0]);
    }
}

// ========== Task 44: Query Parameters ==========
