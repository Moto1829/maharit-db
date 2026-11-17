//! 制約（UNIQUE / NOT NULL / 型 / ラベル）と EXPLAIN / PROFILE。

use super::*;

#[test]
fn test_create_unique_constraint() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &["CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE"],
    );
    assert!(results[0].is_ok());
    let rs = results[0].as_ref().unwrap();
    assert_eq!(
        rs.rows[0].columns[0],
        Value::String("Constraint 'unique_email' created".to_string())
    );
}

#[test]
fn test_create_not_null_constraint() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &["CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL"],
    );
    assert!(results[0].is_ok());
}

#[test]
fn test_create_type_constraint() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &["CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER"],
    );
    assert!(results[0].is_ok());
}

#[test]
fn test_drop_constraint() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
            "DROP CONSTRAINT unique_email",
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    let rs = results[1].as_ref().unwrap();
    assert_eq!(
        rs.rows[0].columns[0],
        Value::String("Constraint 'unique_email' dropped".to_string())
    );
}

#[test]
fn test_drop_nonexistent_constraint() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(&mut graph, &["DROP CONSTRAINT nonexistent"]);
    assert!(matches!(results[0], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_show_constraints() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
            "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
            "SHOW CONSTRAINTS",
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok());
    let rs = results[2].as_ref().unwrap();
    assert_eq!(rs.columns, vec!["name", "label", "properties", "type"]);
    assert_eq!(rs.rows.len(), 2);
}

#[test]
fn test_show_constraints_empty() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(&mut graph, &["SHOW CONSTRAINTS"]);
    assert!(results[0].is_ok());
    let rs = results[0].as_ref().unwrap();
    assert_eq!(rs.rows.len(), 0);
}

#[test]
fn test_unique_constraint_enforced_on_create() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
            r#"CREATE (n:Person {email: "alice@example.com"})"#,
            r#"CREATE (n:Person {email: "alice@example.com"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(matches!(results[2], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_not_null_constraint_enforced_on_create() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
            r#"CREATE (n:Person {age: 30})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_type_constraint_enforced_on_create() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER",
            r#"CREATE (n:Person {age: "thirty"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_type_constraint_allows_valid_type() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER",
            r#"CREATE (n:Person {age: 30})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
}

#[test]
fn test_unique_constraint_enforced_on_set() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
            r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
            r#"CREATE (n:Person {name: "Bob", email: "bob@example.com"})"#,
            r#"MATCH (n:Person {name: "Bob"}) SET n.email = "alice@example.com""#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok());
    assert!(matches!(results[3], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_not_null_constraint_enforced_on_remove() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL",
            r#"CREATE (n:Person {name: "Alice"})"#,
            r#"MATCH (n:Person {name: "Alice"}) REMOVE n.name"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(matches!(results[2], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_constraint_different_label_not_enforced() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE",
            r#"CREATE (n:Person {email: "shared@example.com"})"#,
            r#"CREATE (n:Company {email: "shared@example.com"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok()); // different label, should pass
}

#[test]
fn test_composite_unique_constraint_enforced() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE",
            r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
            r#"CREATE (n:Person {name: "Alice", email: "different@example.com"})"#, // Different email, should pass
            r#"CREATE (n:Person {name: "Bob", email: "alice@example.com"})"#, // Different name, should pass
            r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#, // Same combo, should fail
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok()); // different combo, should pass
    assert!(results[3].is_ok()); // different combo, should pass
    assert!(matches!(results[4], Err(ExecuteError::ConstraintError(_)))); // duplicate combo
}

#[test]
fn test_composite_unique_constraint_missing_property() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE",
            r#"CREATE (n:Person {name: "Alice", email: "alice@example.com"})"#,
            r#"CREATE (n:Person {name: "Bob"})"#, // Missing email, constraint doesn't apply
            r#"CREATE (n:Person {email: "alice@example.com"})"#, // Missing name, constraint doesn't apply
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok()); // missing property, should pass
    assert!(results[3].is_ok()); // missing property, should pass
}

#[test]
fn test_parse_composite_unique_constraint() {
    let stmt = Parser::new(
        "CREATE CONSTRAINT unique_name_email FOR (n:Person) REQUIRE (n.name, n.email) IS UNIQUE",
    )
    .unwrap()
    .parse()
    .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(cc.name, "unique_name_email");
        assert_eq!(cc.label, "Person");
        assert_eq!(cc.properties, vec!["name".to_string(), "email".to_string()]);
        assert_eq!(cc.constraint_type, ConstraintTypeAst::Unique);
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_duplicate_constraint_name_error() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT c1 FOR (n:Person) REQUIRE n.email IS UNIQUE",
            "CREATE CONSTRAINT c1 FOR (n:Person) REQUIRE n.name IS NOT NULL",
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_parse_create_constraint() {
    let stmt =
        Parser::new("CREATE CONSTRAINT unique_email FOR (n:Person) REQUIRE n.email IS UNIQUE")
            .unwrap()
            .parse()
            .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(cc.name, "unique_email");
        assert_eq!(cc.label, "Person");
        assert_eq!(cc.properties, vec!["email".to_string()]);
        assert_eq!(cc.constraint_type, ConstraintTypeAst::Unique);
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_parse_create_constraint_not_null() {
    let stmt =
        Parser::new("CREATE CONSTRAINT require_name FOR (n:Person) REQUIRE n.name IS NOT NULL")
            .unwrap()
            .parse()
            .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(cc.constraint_type, ConstraintTypeAst::NotNull);
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_parse_create_constraint_type_check() {
    let stmt = Parser::new("CREATE CONSTRAINT age_type FOR (n:Person) REQUIRE n.age IS :: INTEGER")
        .unwrap()
        .parse()
        .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(
            cc.constraint_type,
            ConstraintTypeAst::TypeCheck(PropertyTypeAst::Integer)
        );
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_parse_drop_constraint() {
    let stmt = Parser::new("DROP CONSTRAINT unique_email")
        .unwrap()
        .parse()
        .unwrap();
    if let Statement::DropConstraint(dc) = stmt {
        assert_eq!(dc.name, "unique_email");
    } else {
        panic!("expected DropConstraint statement");
    }
}

#[test]
fn test_parse_show_constraints() {
    let stmt = Parser::new("SHOW CONSTRAINTS").unwrap().parse().unwrap();
    assert_eq!(stmt, Statement::ShowConstraints);
}

// ========== Label constraint tests ==========

#[test]
fn test_parse_required_label_constraint() {
    let stmt =
        Parser::new("CREATE CONSTRAINT employee_is_person FOR (n:Employee) REQUIRE n:Person")
            .unwrap()
            .parse()
            .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(cc.name, "employee_is_person");
        assert_eq!(cc.label, "Employee");
        assert_eq!(cc.properties, Vec::<String>::new());
        assert_eq!(
            cc.constraint_type,
            ConstraintTypeAst::RequiredLabel("Person".to_string())
        );
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_required_label_constraint_pass() {
    let mut graph = Graph::new();
    // Employee requires Person label; since our graph model has single labels,
    // this passes when the node's label matches the required label (Person)
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
            // Creating a Person node satisfies the Employee->Person constraint
            // (the constraint says: if label=Employee, require label=Person)
            // Since Person != Employee, creating Employee would fail
            r#"CREATE (n:Person {name: "Alice"})"#, // Person node, not Employee - not affected by constraint
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok()); // Person node bypasses Employee constraint
}

#[test]
fn test_required_label_constraint_violation() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
            r#"CREATE (n:Employee {name: "Bob"})"#, // Employee but not Person - violates constraint
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_required_label_constraint_satisfied_by_same_label() {
    let mut graph = Graph::new();
    // If required label equals the node label, the constraint is trivially satisfied
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT person_is_person FOR (n:Person) REQUIRE n:Person",
            r#"CREATE (n:Person {name: "Alice"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok()); // Person satisfies Person requirement
}

#[test]
fn test_parse_endpoint_label_constraint() {
    let stmt = Parser::new("CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)")
        .unwrap()
        .parse()
        .unwrap();
    if let Statement::CreateConstraint(cc) = stmt {
        assert_eq!(cc.name, "knows_persons");
        assert_eq!(cc.label, "KNOWS");
        assert_eq!(cc.properties, Vec::<String>::new());
        assert_eq!(
            cc.constraint_type,
            ConstraintTypeAst::EndpointLabel {
                source_label: "Person".to_string(),
                target_label: "Person".to_string(),
            }
        );
    } else {
        panic!("expected CreateConstraint statement");
    }
}

#[test]
fn test_endpoint_label_constraint_violation() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
            r#"CREATE (p:Person {name: "Alice"})-[:KNOWS]->(c:Company {name: "Acme"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_endpoint_label_constraint_pass() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
            r#"CREATE (p:Person {name: "Alice"})-[:KNOWS]->(q:Person {name: "Bob"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
}

#[test]
fn test_endpoint_label_constraint_source_violation() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
            r#"CREATE (c:Company {name: "Acme"})-[:KNOWS]->(p:Person {name: "Alice"})"#,
        ],
    );
    assert!(results[0].is_ok());
    assert!(matches!(results[1], Err(ExecuteError::ConstraintError(_))));
}

#[test]
fn test_show_constraints_includes_label_constraints() {
    let mut graph = Graph::new();
    let results = execute_with_constraints(
        &mut graph,
        &[
            "CREATE CONSTRAINT emp_is_person FOR (n:Employee) REQUIRE n:Person",
            "CREATE CONSTRAINT knows_persons FOR (p:Person)-[r:KNOWS]->(q:Person)",
            "SHOW CONSTRAINTS",
        ],
    );
    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_ok());
    let rs = results[2].as_ref().unwrap();
    assert_eq!(rs.rows.len(), 2);
    // Both constraints should be listed
    let names: Vec<String> = rs
        .rows
        .iter()
        .map(|r| {
            if let Value::String(s) = &r.columns[0] {
                s.clone()
            } else {
                String::new()
            }
        })
        .collect();
    assert!(names.contains(&"emp_is_person".to_string()));
    assert!(names.contains(&"knows_persons".to_string()));
}

// ========== EXPLAIN / PROFILE tests ==========

#[test]
fn test_explain_match() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = execute(&mut graph, "EXPLAIN MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.columns, vec!["plan"]);
    assert!(!result.rows.is_empty());

    // Check that plan contains expected operators
    let plan_text: String = result
        .rows
        .iter()
        .map(|r| {
            if let Value::String(s) = &r.columns[0] {
                s.clone()
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan_text.contains("NodeByLabelScan"));
    assert!(plan_text.contains("Projection"));
}

#[test]
fn test_explain_match_with_filter() {
    let mut graph = Graph::new();
    let result = execute(
        &mut graph,
        "EXPLAIN MATCH (n:Person) WHERE n.age > 30 RETURN n",
    )
    .unwrap();
    let plan_text: String = result
        .rows
        .iter()
        .filter_map(|r| {
            if let Value::String(s) = &r.columns[0] {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan_text.contains("NodeByLabelScan"));
    assert!(plan_text.contains("Filter"));
    assert!(plan_text.contains("Projection"));
}

#[test]
fn test_explain_create() {
    let mut graph = Graph::new();
    let result = execute(&mut graph, r#"EXPLAIN CREATE (n:Person {name: "Alice"})"#).unwrap();
    let plan_text: String = result
        .rows
        .iter()
        .filter_map(|r| {
            if let Value::String(s) = &r.columns[0] {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan_text.contains("CreateNode"));
    // EXPLAIN should NOT actually create the node
    assert_eq!(graph.node_count(), 0);
}

#[test]
fn test_explain_does_not_mutate_graph() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    assert_eq!(graph.node_count(), 1);

    // EXPLAIN should not add/remove nodes
    execute(&mut graph, r#"EXPLAIN CREATE (m:Person {name: "Bob"})"#).unwrap();
    assert_eq!(graph.node_count(), 1);
}

#[test]
fn test_profile_match() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Person {name: "Bob", age: 25})"#).unwrap();

    let result = execute(&mut graph, "PROFILE MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.columns, vec!["profile"]);
    assert!(!result.rows.is_empty());

    let plan_text: String = result
        .rows
        .iter()
        .filter_map(|r| {
            if let Value::String(s) = &r.columns[0] {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // PROFILE output should contain actual execution stats
    assert!(plan_text.contains("Rows:"));
    assert!(plan_text.contains("Time:"));
}

#[test]
fn test_profile_executes_query() {
    let mut graph = Graph::new();
    // PROFILE CREATE should actually create the node (unlike EXPLAIN)
    let result = execute(&mut graph, r#"PROFILE CREATE (n:Person {name: "Alice"})"#).unwrap();
    assert_eq!(graph.node_count(), 1);
    assert!(!result.rows.is_empty());
}

#[test]
fn test_explain_path_pattern() {
    let mut graph = Graph::new();
    let result = execute(
        &mut graph,
        "EXPLAIN MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b",
    )
    .unwrap();
    let plan_text: String = result
        .rows
        .iter()
        .filter_map(|r| {
            if let Value::String(s) = &r.columns[0] {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan_text.contains("NodeByLabelScan"));
    assert!(plan_text.contains("Expand"));
}

#[test]
fn test_explain_merge() {
    let mut graph = Graph::new();
    let result = execute(&mut graph, r#"EXPLAIN MERGE (n:Person {name: "Alice"})"#).unwrap();
    let plan_text: String = result
        .rows
        .iter()
        .filter_map(|r| {
            if let Value::String(s) = &r.columns[0] {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan_text.contains("Merge"));
    // EXPLAIN should not create the node
    assert_eq!(graph.node_count(), 0);
}

// ========== CONTAINS operator tests ==========
