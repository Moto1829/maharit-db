//! MATCH と組み合わせた CREATE / SET / MERGE / REMOVE と UNWIND の構文解析。

use super::*;

#[test]
fn test_parse_match_create() {
    let stmt = parse(
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    if let Statement::MatchCreate(mc) = stmt {
        assert_eq!(mc.create_clause.patterns.len(), 1);
    } else {
        panic!("expected MatchCreate statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_match_create_with_where() {
    let stmt = parse(
        r#"MATCH (a:Person) WHERE a.age > 20 CREATE (a)-[:MEMBER_OF]->(g:Group {name: "Adults"})"#,
    )
    .unwrap();

    if let Statement::MatchCreate(mc) = stmt {
        assert!(mc.where_clause.is_some());
        assert_eq!(mc.create_clause.patterns.len(), 1);
    } else {
        panic!("expected MatchCreate statement");
    }
}

// ========== MATCH + SET tests ==========

#[test]
fn test_parse_match_set_standalone() {
    let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31 RETURN n"#).unwrap();

    if let Statement::MatchSet(ms) = stmt {
        assert_eq!(ms.set_clause.items.len(), 1);
        assert!(ms.return_clause.is_some());
    } else {
        panic!("expected MatchSet statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_match_set_multiple() {
    let stmt =
        parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31, n.city = "Tokyo" RETURN n"#)
            .unwrap();

    if let Statement::MatchSet(ms) = stmt {
        assert_eq!(ms.set_clause.items.len(), 2);
    } else {
        panic!("expected MatchSet statement");
    }
}

#[test]
fn test_parse_match_set_no_return() {
    let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) SET n.age = 31"#).unwrap();

    if let Statement::MatchSet(ms) = stmt {
        assert!(ms.return_clause.is_none());
    } else {
        panic!("expected MatchSet statement");
    }
}

// ========== MERGE tests ==========

#[test]
fn test_parse_merge_simple() {
    let stmt = parse(r#"MERGE (n:Person {name: "Alice"})"#).unwrap();

    if let Statement::Merge(m) = stmt {
        assert!(m.match_clauses.is_empty());
        assert_eq!(m.patterns.len(), 1);
        assert!(m.on_create_set.is_none());
        assert!(m.on_match_set.is_none());
    } else {
        panic!("expected Merge statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_merge_with_on_create_set() {
    let stmt = parse(r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25"#).unwrap();

    if let Statement::Merge(m) = stmt {
        assert!(m.on_create_set.is_some());
        assert!(m.on_match_set.is_none());
    } else {
        panic!("expected Merge statement");
    }
}

#[test]
fn test_parse_merge_with_both_on_clauses() {
    let stmt = parse(
        r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.age = 25 ON MATCH SET n.age = 30"#,
    )
    .unwrap();

    if let Statement::Merge(m) = stmt {
        assert!(m.on_create_set.is_some());
        assert!(m.on_match_set.is_some());
    } else {
        panic!("expected Merge statement");
    }
}

#[test]
fn test_parse_match_merge() {
    let stmt = parse(
        r#"MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) MERGE (a)-[:KNOWS]->(b)"#,
    )
    .unwrap();

    if let Statement::Merge(m) = stmt {
        assert_eq!(m.match_clauses.len(), 1);
        assert_eq!(m.patterns.len(), 1);
    } else {
        panic!("expected Merge statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_merge_with_return() {
    let stmt = parse(r#"MERGE (n:Person {name: "Alice"}) RETURN n"#).unwrap();

    if let Statement::Merge(m) = stmt {
        assert!(m.return_clause.is_some());
    } else {
        panic!("expected Merge statement");
    }
}

// ========== MATCH + REMOVE tests ==========

#[test]
fn test_parse_match_remove_property() {
    let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age RETURN n"#).unwrap();

    if let Statement::MatchRemove(mr) = stmt {
        assert_eq!(mr.remove_clause.items.len(), 1);
        assert!(matches!(
            mr.remove_clause.items[0],
            RemoveItem::Property(_, _)
        ));
        assert!(mr.return_clause.is_some());
    } else {
        panic!("expected MatchRemove statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_match_remove_label() {
    let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n:Person RETURN n"#).unwrap();

    if let Statement::MatchRemove(mr) = stmt {
        assert_eq!(mr.remove_clause.items.len(), 1);
        if let RemoveItem::Label(var, label) = &mr.remove_clause.items[0] {
            assert_eq!(var, "n");
            assert_eq!(label, "Person");
        } else {
            panic!("expected Label remove item");
        }
    } else {
        panic!("expected MatchRemove statement");
    }
}

#[test]
fn test_parse_match_remove_multiple() {
    let stmt = parse(r#"MATCH (n:Person {name: "Alice"}) REMOVE n.age, n.city RETURN n"#).unwrap();

    if let Statement::MatchRemove(mr) = stmt {
        assert_eq!(mr.remove_clause.items.len(), 2);
    } else {
        panic!("expected MatchRemove statement");
    }
}

// ========== UNWIND tests ==========

#[test]
fn test_parse_unwind_list() {
    let stmt = parse("UNWIND [1, 2, 3] AS x RETURN x").unwrap();

    if let Statement::Unwind(uw) = stmt {
        assert_eq!(uw.variable, "x");
        assert!(uw.return_clause.is_some());
        assert!(uw.create_clause.is_none());
    } else {
        panic!("expected Unwind statement, got {:?}", stmt);
    }
}

#[test]
fn test_parse_unwind_string_list() {
    let stmt = parse(r#"UNWIND ["a", "b"] AS name RETURN name"#).unwrap();

    if let Statement::Unwind(uw) = stmt {
        assert_eq!(uw.variable, "name");
    } else {
        panic!("expected Unwind statement");
    }
}

#[test]
fn test_parse_list_expression() {
    let stmt = parse("UNWIND [1, 2, 3] AS x RETURN x").unwrap();

    if let Statement::Unwind(uw) = stmt {
        if let Expression::List(elems) = &uw.expression {
            assert_eq!(elems.len(), 3);
        } else {
            panic!("expected List expression");
        }
    } else {
        panic!("expected Unwind statement");
    }
}

// ========== Lexer keyword tests ==========
