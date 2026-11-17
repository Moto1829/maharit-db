//! date / datetime / duration と時間型の演算・フィールド。

use super::*;

#[test]
fn test_date_func_no_arg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN date()").unwrap();
    match &result.rows[0].columns[0] {
        Value::Date(d) => assert!(*d > 0, "date should be after epoch"),
        other => panic!("expected Date, got {:?}", other),
    }
}

#[test]
fn test_date_func_string_arg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN date(\"2024-01-15\")").unwrap();
    match &result.rows[0].columns[0] {
        Value::Date(d) => {
            let (y, m, day) = maharit_core::temporal::days_to_ymd(*d);
            assert_eq!(y, 2024);
            assert_eq!(m, 1);
            assert_eq!(day, 15);
        }
        other => panic!("expected Date, got {:?}", other),
    }
}

#[test]
fn test_datetime_func_no_arg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN datetime()").unwrap();
    match &result.rows[0].columns[0] {
        Value::DateTime(ms) => assert!(*ms > 0),
        other => panic!("expected DateTime, got {:?}", other),
    }
}

#[test]
fn test_datetime_func_string_arg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN datetime(\"2024-06-15T12:30:00Z\")",
    )
    .unwrap();
    match &result.rows[0].columns[0] {
        Value::DateTime(ms) => {
            let (y, mo, d, h, mi, s, _) = maharit_core::temporal::millis_to_datetime(*ms);
            assert_eq!(y, 2024);
            assert_eq!(mo, 6);
            assert_eq!(d, 15);
            assert_eq!(h, 12);
            assert_eq!(mi, 30);
            assert_eq!(s, 0);
        }
        other => panic!("expected DateTime, got {:?}", other),
    }
}

#[test]
fn test_duration_func_string_arg() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN duration(\"P1Y2M3D\")").unwrap();
    match &result.rows[0].columns[0] {
        Value::Duration {
            months,
            days,
            millis,
        } => {
            assert_eq!(*months, 14); // 1*12 + 2
            assert_eq!(*days, 3);
            assert_eq!(*millis, 0);
        }
        other => panic!("expected Duration, got {:?}", other),
    }
}

#[test]
fn test_duration_with_time() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(&mut graph, "MATCH (n:T) RETURN duration(\"PT2H30M\")").unwrap();
    match &result.rows[0].columns[0] {
        Value::Duration {
            months,
            days,
            millis,
        } => {
            assert_eq!(*months, 0);
            assert_eq!(*days, 0);
            assert_eq!(*millis, 2 * 3_600_000 + 30 * 60_000);
        }
        other => panic!("expected Duration, got {:?}", other),
    }
}

#[test]
fn test_date_comparison() {
    let mut graph = Graph::new();
    let nid = graph.create_node("Event");
    graph.get_node_mut(nid).unwrap().set_property(
        "date",
        PropertyValue::Date(maharit_core::temporal::ymd_to_days(2024, 6, 15)),
    );
    // Should match: 2024-06-15 >= 2024-01-01
    let result = execute(
        &mut graph,
        "MATCH (e:Event) WHERE e.date >= date(\"2024-01-01\") RETURN e.date",
    )
    .unwrap();
    assert_eq!(result.row_count(), 1);
    // Should not match: 2024-06-15 >= 2025-01-01
    let result2 = execute(
        &mut graph,
        "MATCH (e:Event) WHERE e.date >= date(\"2025-01-01\") RETURN e.date",
    )
    .unwrap();
    assert_eq!(result2.row_count(), 0);
}

#[test]
fn test_date_arithmetic() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    // date("2024-01-01") + duration("P1D") should be 2024-01-02
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN date(\"2024-01-01\") + duration(\"P1D\")",
    )
    .unwrap();
    match &result.rows[0].columns[0] {
        Value::Date(d) => {
            let (y, m, day) = maharit_core::temporal::days_to_ymd(*d);
            assert_eq!(y, 2024);
            assert_eq!(m, 1);
            assert_eq!(day, 2);
        }
        other => panic!("expected Date, got {:?}", other),
    }
}

#[test]
fn test_date_subtraction() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    // date("2024-01-10") - date("2024-01-01") = duration of 9 days
    let result = execute(
        &mut graph,
        "MATCH (n:T) RETURN date(\"2024-01-10\") - date(\"2024-01-01\")",
    )
    .unwrap();
    match &result.rows[0].columns[0] {
        Value::Duration {
            months: 0,
            days: 9,
            millis: 0,
        } => {}
        other => panic!("expected Duration{{days:9}}, got {:?}", other),
    }
}

#[test]
fn test_temporal_create_and_persist() {
    let mut graph = Graph::new();
    let days = maharit_core::temporal::ymd_to_days(2024, 3, 15);
    let nid = graph.create_node("Event");
    graph
        .get_node_mut(nid)
        .unwrap()
        .set_property("date", PropertyValue::Date(days));
    let node = graph.get_node(nid).unwrap();
    assert_eq!(node.get_property("date"), Some(&PropertyValue::Date(days)));
}

#[test]
fn test_date_field_accessors() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:T) WITH date("2024-06-15") AS d RETURN d.year, d.month, d.day"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2024));
    assert_eq!(result.rows[0].columns[1], Value::Int(6));
    assert_eq!(result.rows[0].columns[2], Value::Int(15));
}

#[test]
fn test_datetime_field_accessors() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:T) WITH datetime("2024-06-15T10:30:45Z") AS dt RETURN dt.year, dt.hour, dt.minute, dt.second"#,
    ).unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2024));
    assert_eq!(result.rows[0].columns[1], Value::Int(10));
    assert_eq!(result.rows[0].columns[2], Value::Int(30));
    assert_eq!(result.rows[0].columns[3], Value::Int(45));
}

#[test]
fn test_duration_field_accessors() {
    // P1Y2M3DT4H5M6S = 1 year, 2 months, 3 days, 4 hours, 5 minutes, 6 seconds
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        r#"MATCH (n:T) WITH duration("P1Y2M3DT4H5M6S") AS dur RETURN dur.years, dur.months, dur.days, dur.hours, dur.minutes, dur.seconds"#,
    ).unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(1)); // years
    assert_eq!(result.rows[0].columns[1], Value::Int(2)); // months (within year)
    assert_eq!(result.rows[0].columns[2], Value::Int(3)); // days
    assert_eq!(result.rows[0].columns[3], Value::Int(4)); // hours
    assert_eq!(result.rows[0].columns[4], Value::Int(5)); // minutes
    assert_eq!(result.rows[0].columns[5], Value::Int(6)); // seconds
}

#[test]
fn test_date_from_map() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:T) WITH date({year: 2024, month: 3, day: 7}) AS d RETURN d.year, d.month, d.day",
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(2024));
    assert_eq!(result.rows[0].columns[1], Value::Int(3));
    assert_eq!(result.rows[0].columns[2], Value::Int(7));
}

#[test]
fn test_duration_from_map() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (n:T)").unwrap();
    let result = execute(
        &mut graph,
        "MATCH (n:T) WITH duration({years: 1, months: 2, days: 3, hours: 4, minutes: 5, seconds: 6}) AS dur RETURN dur.years, dur.months, dur.days, dur.hours",
    ).unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(1));
    assert_eq!(result.rows[0].columns[1], Value::Int(2));
    assert_eq!(result.rows[0].columns[2], Value::Int(3));
    assert_eq!(result.rows[0].columns[3], Value::Int(4));
}

#[test]
fn test_date_field_in_where_clause() {
    let mut graph = Graph::new();
    let nid = graph.create_node_with_labels(vec!["Event".to_string()]);
    let days = maharit_core::temporal::ymd_to_days(2024, 6, 15);
    graph
        .get_node_mut(nid)
        .unwrap()
        .set_property("date", PropertyValue::Date(days));
    // MATCH event, carry date through WITH, filter on year, return month
    let result = execute(
        &mut graph,
        r#"MATCH (e:Event) WITH e.date AS d WHERE d.year = 2024 RETURN d.month"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns[0], Value::Int(6));
}

// ========== PROPERTY INDEX TESTS ==========
