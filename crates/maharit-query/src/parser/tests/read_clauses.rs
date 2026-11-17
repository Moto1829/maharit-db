//! CREATE / MATCH / DELETE / SET、可変長パス、最短経路、OPTIONAL MATCH、CASE、WITH、正規表現、UNION の構文解析。

use super::*;

#[test]
fn create_index_for_on_syntax() {
    let parse = |q: &str| Parser::new(q).unwrap().parse();
    let expected = Statement::CreateIndex(CreateIndexStatement {
        label: "Person".into(),
        property: "email".into(),
    });
    assert_eq!(
        parse("CREATE INDEX FOR (n:Person) ON (n.email)").unwrap(),
        expected
    );
    assert_eq!(parse("CREATE INDEX ON :Person(email)").unwrap(), expected);
    assert!(parse("CREATE INDEX FOR (n:Person) ON (m.email)").is_err());
    assert!(parse("CREATE INDEX FOR (n:Person) ON (n.a, n.b)").is_err());
}

#[test]
fn test_create_simple_node() {
    let stmt = parse("CREATE (n)").unwrap();

    if let Statement::Create(create) = stmt {
        assert_eq!(create.patterns.len(), 1);
        if let Pattern::Node(node) = &create.patterns[0] {
            assert_eq!(node.variable, Some("n".to_string()));
            assert!(node.labels.is_empty());
        } else {
            panic!("expected node pattern");
        }
    } else {
        panic!("expected CREATE statement");
    }
}

#[test]
fn test_create_node_with_label() {
    let stmt = parse("CREATE (n:Person)").unwrap();

    if let Statement::Create(create) = stmt {
        if let Pattern::Node(node) = &create.patterns[0] {
            assert_eq!(node.variable, Some("n".to_string()));
            assert_eq!(node.labels, vec!["Person".to_string()]);
        } else {
            panic!("expected node pattern");
        }
    } else {
        panic!("expected CREATE statement");
    }
}

#[test]
fn test_create_node_with_multiple_labels() {
    let stmt = parse("CREATE (n:Person:Employee)").unwrap();

    if let Statement::Create(create) = stmt {
        if let Pattern::Node(node) = &create.patterns[0] {
            assert_eq!(node.variable, Some("n".to_string()));
            assert_eq!(
                node.labels,
                vec!["Person".to_string(), "Employee".to_string()]
            );
        } else {
            panic!("expected node pattern");
        }
    } else {
        panic!("expected CREATE statement");
    }
}

#[test]
fn test_create_node_with_properties() {
    let stmt = parse(r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    if let Statement::Create(create) = stmt {
        if let Pattern::Node(node) = &create.patterns[0] {
            assert_eq!(node.variable, Some("n".to_string()));
            assert_eq!(node.labels, vec!["Person".to_string()]);
            assert_eq!(
                node.properties.get("name"),
                Some(&Expression::Literal(Literal::String("Alice".to_string())))
            );
            assert_eq!(
                node.properties.get("age"),
                Some(&Expression::Literal(Literal::Int(30)))
            );
        } else {
            panic!("expected node pattern");
        }
    } else {
        panic!("expected CREATE statement");
    }
}

#[test]
fn test_create_path() {
    let stmt = parse("CREATE (a:Person)-[:KNOWS]->(b:Person)").unwrap();

    if let Statement::Create(create) = stmt {
        if let Pattern::Path(path) = &create.patterns[0] {
            assert_eq!(path.start.variable, Some("a".to_string()));
            assert_eq!(path.start.labels, vec!["Person".to_string()]);
            assert_eq!(path.segments.len(), 1);

            let seg = &path.segments[0];
            assert_eq!(seg.edge.edge_types, vec!["KNOWS".to_string()]);
            assert_eq!(seg.edge.direction, EdgeDirection::Outgoing);
            assert_eq!(seg.node.variable, Some("b".to_string()));
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected CREATE statement");
    }
}

#[test]
fn test_match_return() {
    let stmt = parse("MATCH (n:Person) RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.segments[0].match_clauses[0].patterns.len(), 1);
        assert!(m.segments[0].where_clause.is_none());
        assert_eq!(m.return_clause.items.len(), 1);
        assert_eq!(
            m.return_clause.items[0],
            ReturnItem::Variable("n".to_string())
        );
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_match_return_property() {
    let stmt = parse("MATCH (n:Person) RETURN n.name").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(
            m.return_clause.items[0],
            ReturnItem::Property("n".to_string(), "name".to_string())
        );
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_match_where() {
    let stmt = parse("MATCH (n:Person) WHERE n.age > 18 RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        let where_clause = m.segments[0].where_clause.clone().unwrap();
        if let Expression::BinaryOp(left, op, right) = where_clause {
            assert_eq!(op, BinaryOp::Gt);
            assert_eq!(
                *left,
                Expression::Property("n".to_string(), "age".to_string())
            );
            assert_eq!(*right, Expression::Literal(Literal::Int(18)));
        } else {
            panic!("expected binary expression");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_match_where_and() {
    let stmt = parse(r#"MATCH (n:Person) WHERE n.age > 18 AND n.name = "Alice" RETURN n"#).unwrap();

    if let Statement::Match(m) = stmt {
        let where_clause = m.segments[0].where_clause.clone().unwrap();
        if let Expression::BinaryOp(_, op, _) = where_clause {
            assert_eq!(op, BinaryOp::And);
        } else {
            panic!("expected AND expression");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_return_multiple() {
    let stmt = parse("MATCH (n:Person) RETURN n.name, n.age").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 2);
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_return_star() {
    let stmt = parse("MATCH (n:Person) RETURN *").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items[0], ReturnItem::All);
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== DELETE tests ==========

#[test]
fn test_delete_node() {
    let stmt = parse("MATCH (n:Person) DELETE n").unwrap();

    if let Statement::Delete(d) = stmt {
        assert_eq!(d.patterns.len(), 1);
        assert!(d.where_clause.is_none());
        assert!(d.set_clause.is_none());
        assert!(!d.delete_clause.detach);
        assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
    } else {
        panic!("expected DELETE statement");
    }
}

#[test]
fn test_delete_with_where() {
    let stmt = parse(r#"MATCH (n:Person) WHERE n.name = "Alice" DELETE n"#).unwrap();

    if let Statement::Delete(d) = stmt {
        assert!(d.where_clause.is_some());
        assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
    } else {
        panic!("expected DELETE statement");
    }
}

#[test]
fn test_detach_delete() {
    let stmt = parse("MATCH (n:Person) DETACH DELETE n").unwrap();

    if let Statement::Delete(d) = stmt {
        assert!(d.delete_clause.detach);
        assert_eq!(d.delete_clause.variables, vec!["n".to_string()]);
    } else {
        panic!("expected DELETE statement");
    }
}

#[test]
fn test_delete_multiple() {
    let stmt = parse("MATCH (a)-[r]->(b) DELETE a, r, b").unwrap();

    if let Statement::Delete(d) = stmt {
        assert_eq!(
            d.delete_clause.variables,
            vec!["a".to_string(), "r".to_string(), "b".to_string()]
        );
    } else {
        panic!("expected DELETE statement");
    }
}

#[test]
fn test_delete_edge() {
    let stmt = parse("MATCH (a)-[r:KNOWS]->(b) DELETE r").unwrap();

    if let Statement::Delete(d) = stmt {
        assert!(!d.delete_clause.detach);
        assert_eq!(d.delete_clause.variables, vec!["r".to_string()]);
    } else {
        panic!("expected DELETE statement");
    }
}

// ========== SET tests ==========

#[test]
fn test_set_property() {
    let stmt = parse(r#"MATCH (n:Person) SET n.age = 31 DELETE n"#).unwrap();

    if let Statement::Delete(d) = stmt {
        let set = d.set_clause.unwrap();
        assert_eq!(set.items.len(), 1);
        assert_eq!(
            set.items[0],
            SetItem::Property(
                "n".to_string(),
                "age".to_string(),
                Expression::Literal(Literal::Int(31))
            )
        );
    } else {
        panic!("expected DELETE statement with SET");
    }
}

#[test]
fn test_set_multiple_properties() {
    let stmt = parse(r#"MATCH (n:Person) SET n.age = 31, n.name = "Bob" DELETE n"#).unwrap();

    if let Statement::Delete(d) = stmt {
        let set = d.set_clause.unwrap();
        assert_eq!(set.items.len(), 2);
        assert!(matches!(&set.items[0], SetItem::Property(v, p, _) if v == "n" && p == "age"));
        assert!(matches!(&set.items[1], SetItem::Property(v, p, _) if v == "n" && p == "name"));
    } else {
        panic!("expected DELETE statement with SET");
    }
}

// ========== Variable-length path tests ==========

#[test]
fn test_variable_length_path_exact() {
    let stmt = parse("MATCH (a)-[:KNOWS*2]->(b) RETURN a").unwrap();

    if let Statement::Match(m) = stmt {
        if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
            let seg = &path.segments[0];
            assert_eq!(seg.edge.edge_types, vec!["KNOWS".to_string()]);
            let range = seg
                .edge
                .length_range
                .as_ref()
                .expect("should have length_range");
            assert_eq!(range.min, 2);
            assert_eq!(range.max, Some(2));
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_variable_length_path_range() {
    let stmt = parse("MATCH (a)-[:KNOWS*2..5]->(b) RETURN a").unwrap();

    if let Statement::Match(m) = stmt {
        if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
            let seg = &path.segments[0];
            let range = seg
                .edge
                .length_range
                .as_ref()
                .expect("should have length_range");
            assert_eq!(range.min, 2);
            assert_eq!(range.max, Some(5));
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_variable_length_path_unlimited() {
    let stmt = parse("MATCH (a)-[:KNOWS*2..]->(b) RETURN a").unwrap();

    if let Statement::Match(m) = stmt {
        if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
            let seg = &path.segments[0];
            let range = seg
                .edge
                .length_range
                .as_ref()
                .expect("should have length_range");
            assert_eq!(range.min, 2);
            assert_eq!(range.max, None);
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_parse_nodes_function() {
    let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN nodes(r)").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 1);
        if let ReturnItem::Function(ScalarFunction::Nodes(var)) = &m.return_clause.items[0] {
            assert_eq!(var, "r");
        } else {
            panic!("expected nodes() function");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_parse_relationships_function() {
    let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN relationships(r)").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 1);
        if let ReturnItem::Function(ScalarFunction::Relationships(var)) = &m.return_clause.items[0]
        {
            assert_eq!(var, "r");
        } else {
            panic!("expected relationships() function");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_parse_length_function() {
    let stmt = parse("MATCH (a)-[r:KNOWS*2]->(b) RETURN length(r)").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 1);
        if let ReturnItem::Function(ScalarFunction::Length(var)) = &m.return_clause.items[0] {
            assert_eq!(var, "r");
        } else {
            panic!("expected length() function");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_variable_length_path_star_only() {
    // [*] should mean 1..unlimited
    let stmt = parse("MATCH (a)-[:KNOWS*]->(b) RETURN a").unwrap();

    if let Statement::Match(m) = stmt {
        if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
            let seg = &path.segments[0];
            let range = seg
                .edge
                .length_range
                .as_ref()
                .expect("should have length_range");
            assert_eq!(range.min, 1);
            assert_eq!(range.max, None); // unlimited
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_variable_length_path_zero_min() {
    // [*0..3] should allow zero hops
    let stmt = parse("MATCH (a)-[:KNOWS*0..3]->(b) RETURN a").unwrap();

    if let Statement::Match(m) = stmt {
        if let Pattern::Path(path) = &m.segments[0].match_clauses[0].patterns[0] {
            let seg = &path.segments[0];
            let range = seg
                .edge
                .length_range
                .as_ref()
                .expect("should have length_range");
            assert_eq!(range.min, 0);
            assert_eq!(range.max, Some(3));
        } else {
            panic!("expected path pattern");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== shortestPath / allShortestPaths tests ==========

#[test]
fn test_parse_shortest_path() {
    let stmt = parse("MATCH (a:Person), (b:Person) RETURN shortestPath(a, b)").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 1);
        if let ReturnItem::Function(ScalarFunction::ShortestPath { start, end }) =
            &m.return_clause.items[0]
        {
            assert_eq!(start, "a");
            assert_eq!(end, "b");
        } else {
            panic!("expected shortestPath() function");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_parse_all_shortest_paths() {
    let stmt = parse("MATCH (a:Person), (b:Person) RETURN allShortestPaths(a, b)").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.return_clause.items.len(), 1);
        if let ReturnItem::Function(ScalarFunction::AllShortestPaths { start, end }) =
            &m.return_clause.items[0]
        {
            assert_eq!(start, "a");
            assert_eq!(end, "b");
        } else {
            panic!("expected allShortestPaths() function");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== OPTIONAL MATCH tests ==========

#[test]
fn test_optional_match() {
    let stmt = parse("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a, b").unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.segments[0].match_clauses.len(), 2);
        assert!(!m.segments[0].match_clauses[0].optional);
        assert!(m.segments[0].match_clauses[1].optional);
        assert_eq!(m.segments[0].match_clauses[0].patterns.len(), 1);
        assert_eq!(m.segments[0].match_clauses[1].patterns.len(), 1);
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_multiple_optional_match() {
    let stmt = parse(
        "MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b) OPTIONAL MATCH (a)-[:WORKS_AT]->(c) RETURN a, b, c",
    )
    .unwrap();

    if let Statement::Match(m) = stmt {
        assert_eq!(m.segments[0].match_clauses.len(), 3);
        assert!(!m.segments[0].match_clauses[0].optional);
        assert!(m.segments[0].match_clauses[1].optional);
        assert!(m.segments[0].match_clauses[2].optional);
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== CASE WHEN tests ==========

#[test]
fn test_case_when_searched() {
    // Searched CASE: CASE WHEN condition THEN result ... END
    let stmt =
        parse("MATCH (n:Person) WHERE CASE WHEN n.age < 20 THEN true ELSE false END RETURN n")
            .unwrap();

    if let Statement::Match(m) = stmt {
        assert!(m.segments[0].where_clause.is_some());
        if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
            assert!(case_expr.operand.is_none()); // Searched CASE
            assert_eq!(case_expr.when_clauses.len(), 1);
            assert!(case_expr.else_clause.is_some());
        } else {
            panic!("expected CASE expression");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_case_when_simple() {
    // Simple CASE: CASE expr WHEN value THEN result ... END
    let stmt = parse(
        "MATCH (n:Person) WHERE CASE n.status WHEN 1 THEN true WHEN 2 THEN false END RETURN n",
    )
    .unwrap();

    if let Statement::Match(m) = stmt {
        assert!(m.segments[0].where_clause.is_some());
        if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
            assert!(case_expr.operand.is_some()); // Simple CASE
            assert_eq!(case_expr.when_clauses.len(), 2);
            assert!(case_expr.else_clause.is_none());
        } else {
            panic!("expected CASE expression");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_case_when_multiple_when() {
    let stmt = parse(
        "MATCH (n:Person) WHERE CASE WHEN n.age < 20 THEN true WHEN n.age < 60 THEN true ELSE false END RETURN n",
    )
    .unwrap();

    if let Statement::Match(m) = stmt {
        if let Expression::Case(case_expr) = m.segments[0].where_clause.as_ref().unwrap() {
            assert_eq!(case_expr.when_clauses.len(), 2);
        } else {
            panic!("expected CASE expression");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== WITH clause tests ==========

#[test]
fn test_with_clause_basic() {
    let stmt = parse("MATCH (n:Person) WITH n RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        // Single segment with MATCH and WITH
        assert_eq!(m.segments.len(), 1);
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert!(!with.distinct);
        assert_eq!(with.items.len(), 1);
        if let ReturnItem::Variable(name) = &with.items[0].expression {
            assert_eq!(name, "n");
        } else {
            panic!("expected variable in WITH");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_clause_alias() {
    let stmt = parse("MATCH (n:Person) WITH n.name AS name RETURN name").unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert_eq!(with.items.len(), 1);
        assert_eq!(with.items[0].alias, Some("name".to_string()));
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_distinct() {
    let stmt = parse("MATCH (n:Person) WITH DISTINCT n.city AS city RETURN city").unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert!(with.distinct);
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_order_by() {
    let stmt = parse("MATCH (n:Person) WITH n ORDER BY n.age RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert!(with.order_by.is_some());
        let order_by = with.order_by.as_ref().unwrap();
        assert_eq!(order_by.items.len(), 1);
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_limit() {
    let stmt = parse("MATCH (n:Person) WITH n LIMIT 10 RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert_eq!(with.limit, Some(Expression::Literal(Literal::Int(10))));
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_skip() {
    let stmt = parse("MATCH (n:Person) WITH n SKIP 5 RETURN n").unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert_eq!(with.skip, Some(Expression::Literal(Literal::Int(5))));
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_aggregation() {
    let stmt = parse("MATCH (n:Person) WITH n.city AS city, COUNT(*) AS count RETURN city, count")
        .unwrap();

    if let Statement::Match(m) = stmt {
        let with = m.segments[0].with_clause.as_ref().unwrap();
        assert_eq!(with.items.len(), 2);
        assert_eq!(with.items[0].alias, Some("city".to_string()));
        assert_eq!(with.items[1].alias, Some("count".to_string()));
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_with_where() {
    let stmt = parse(
        "MATCH (n:Person) WITH n.city AS city, COUNT(*) AS count WHERE count > 5 RETURN city",
    )
    .unwrap();

    if let Statement::Match(m) = stmt {
        // First segment has MATCH and WITH
        // Second segment has WHERE (filtering on WITH results)
        assert_eq!(m.segments.len(), 2);
        assert!(m.segments[0].with_clause.is_some());
        assert!(m.segments[1].where_clause.is_some());
        assert!(m.segments[1].match_clauses.is_empty());
    } else {
        panic!("expected MATCH statement");
    }
}

#[test]
fn test_multiple_with() {
    let stmt = parse("MATCH (n:Person) WITH n.city AS city WITH city RETURN city").unwrap();

    if let Statement::Match(m) = stmt {
        // MATCH + WITH creates 1 segment, second WITH creates another
        assert_eq!(m.segments.len(), 2);
        assert!(m.segments[0].with_clause.is_some());
        assert!(m.segments[1].with_clause.is_some());
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== Regex match (=~) tests ==========

#[test]
fn test_regex_match_parse() {
    let stmt = parse(r#"MATCH (n:Person) WHERE n.name =~ "A.*" RETURN n"#).unwrap();

    if let Statement::Match(m) = stmt {
        let where_clause = m.segments[0].where_clause.clone().unwrap();
        if let Expression::BinaryOp(left, op, right) = where_clause {
            assert_eq!(op, BinaryOp::Regex);
            assert_eq!(
                *left,
                Expression::Property("n".to_string(), "name".to_string())
            );
            assert_eq!(
                *right,
                Expression::Literal(Literal::String("A.*".to_string()))
            );
        } else {
            panic!("expected binary expression with =~");
        }
    } else {
        panic!("expected MATCH statement");
    }
}

// ========== UNION / UNION ALL tests ==========

#[test]
fn test_union_parse() {
    let stmt =
        parse("MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name").unwrap();

    if let Statement::Union(u) = stmt {
        assert_eq!(u.queries.len(), 2);
        assert_eq!(u.union_type, UnionType::Union);
    } else {
        panic!("expected UNION statement");
    }
}

#[test]
fn test_union_all_parse() {
    let stmt =
        parse("MATCH (n:Person) RETURN n.name UNION ALL MATCH (n:Company) RETURN n.name").unwrap();

    if let Statement::Union(u) = stmt {
        assert_eq!(u.queries.len(), 2);
        assert_eq!(u.union_type, UnionType::UnionAll);
    } else {
        panic!("expected UNION statement");
    }
}

#[test]
fn test_union_three_queries() {
    let stmt = parse(
        "MATCH (n:Person) RETURN n.name UNION MATCH (n:Company) RETURN n.name UNION MATCH (n:City) RETURN n.name",
    )
    .unwrap();

    if let Statement::Union(u) = stmt {
        assert_eq!(u.queries.len(), 3);
        assert_eq!(u.union_type, UnionType::Union);
    } else {
        panic!("expected UNION statement");
    }
}

// ========== MATCH + CREATE tests ==========
