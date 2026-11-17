//! CONTAINS と全文索引、プロシージャ呼び出し（CALL … YIELD）。

use super::*;

#[test]
fn test_contains_operator() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {title: "Graph Database Tutorial"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Article {title: "Relational Database Guide"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Article {title: "Machine Learning Basics"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.title CONTAINS "Database" RETURN n.title"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_contains_case_insensitive() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {title: "Graph Database"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.title CONTAINS "database" RETURN n.title"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);

    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.title CONTAINS "DATABASE" RETURN n.title"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

// ========== Fulltext index tests ==========

#[test]
fn test_create_fulltext_index() {
    let mut graph = Graph::new();
    let result = execute(
        &mut graph,
        r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title, n.body)"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    if let Value::String(s) = &result.rows[0].columns[0] {
        assert!(s.contains("article_search"));
    }
}

#[test]
fn test_drop_fulltext_index() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    let stmt = Parser::new(r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title)"#)
        .unwrap()
        .parse()
        .unwrap();
    executor.execute(stmt).unwrap();

    let stmt = Parser::new("DROP FULLTEXT INDEX article_search")
        .unwrap()
        .parse()
        .unwrap();
    let result = executor.execute(stmt).unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_drop_nonexistent_fulltext_index() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    let stmt = Parser::new("DROP FULLTEXT INDEX nonexistent")
        .unwrap()
        .parse()
        .unwrap();
    let result = executor.execute(stmt);
    assert!(result.is_err());
}

#[test]
fn test_create_fulltext_index_indexes_existing_nodes() {
    let mut graph = Graph::new();
    // Create nodes first
    execute(
        &mut graph,
        r#"CREATE (n:Article {title: "Graph Database Tutorial", body: "Learn about graphs"})"#,
    )
    .unwrap();

    // Then create the index - it should index existing nodes
    let stmt =
        Parser::new(r#"CREATE FULLTEXT INDEX article_search FOR (n:Article) ON (n.title, n.body)"#)
            .unwrap()
            .parse()
            .unwrap();
    let mut executor = Executor::new(&mut graph);
    executor.execute(stmt).unwrap();

    // Verify the index has content
    let results = executor
        .fulltext_manager()
        .get_index("article_search")
        .unwrap()
        .search("graph");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_parse_create_fulltext_index() {
    let stmt = Parser::new(r#"CREATE FULLTEXT INDEX my_idx FOR (n:Person) ON (n.name, n.bio)"#)
        .unwrap()
        .parse()
        .unwrap();
    if let Statement::CreateFulltextIndex(cfi) = stmt {
        assert_eq!(cfi.name, "my_idx");
        assert_eq!(cfi.label, "Person");
        assert_eq!(cfi.variable, "n");
        assert_eq!(cfi.properties, vec!["name", "bio"]);
    } else {
        panic!("expected CreateFulltextIndex statement");
    }
}

#[test]
fn test_parse_drop_fulltext_index() {
    let stmt = Parser::new("DROP FULLTEXT INDEX my_idx")
        .unwrap()
        .parse()
        .unwrap();
    if let Statement::DropFulltextIndex(dfi) = stmt {
        assert_eq!(dfi.name, "my_idx");
    } else {
        panic!("expected DropFulltextIndex statement");
    }
}

// ========== Phrase search tests (CONTAINS with quoted phrase) ==========

#[test]
fn test_phrase_search_via_contains_matches_adjacent() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "graph database systems are powerful"})"#,
    )
    .unwrap();
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "database graph systems"})"#,
    )
    .unwrap();

    // Phrase "graph database" should only match the first article
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("graph database systems are powerful".to_string())
    );
}

#[test]
fn test_phrase_search_order_matters() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (n:Article {body: "graph database"})"#).unwrap();
    execute(&mut graph, r#"CREATE (n:Article {body: "database graph"})"#).unwrap();

    // "graph database" must match only the first document (order matters)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("graph database".to_string())
    );
}

#[test]
fn test_phrase_search_no_match_non_adjacent() {
    let mut graph = Graph::new();
    // Words exist but not adjacent
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "graph systems and database management"})"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS '"graph database"' RETURN n.body"#,
    )
    .unwrap();
    assert!(result.rows.is_empty());
}

// ========== Fuzzy search tests (CONTAINS with ~ suffix) ==========

#[test]
fn test_fuzzy_search_via_contains_default_distance() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "database management systems"})"#,
    )
    .unwrap();
    execute(&mut graph, r#"CREATE (n:Article {body: "python code"})"#).unwrap();

    // "dtabase~" has distance 2 from "database" (2 character transpositions)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS 'dtabase~' RETURN n.body"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].columns[0],
        Value::String("database management systems".to_string())
    );
}

#[test]
fn test_fuzzy_search_via_contains_explicit_distance() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "database systems"})"#,
    )
    .unwrap();

    // "databse" has distance 1 from "database"
    // with limit 0, should NOT match
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS 'databse~0' RETURN n.body"#,
    )
    .unwrap();
    assert!(result.rows.is_empty());

    // with limit 1, should match
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS 'databse~1' RETURN n.body"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_fuzzy_search_exact_match_with_tilde() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        r#"CREATE (n:Article {body: "rust programming language"})"#,
    )
    .unwrap();

    // Exact term with ~ should still match (distance 0 <= 2)
    let result = execute(
        &mut graph,
        r#"MATCH (n:Article) WHERE n.body CONTAINS 'rust~' RETURN n.body"#,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
}

// ========== CALL db.index.fulltext.search tests ==========

#[test]
fn test_procedure_call_fulltext_search() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    // Create fulltext index
    executor
        .execute(
            Parser::new(
                r#"CREATE FULLTEXT INDEX article_idx FOR (n:Article) ON (n.title, n.body)"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        )
        .unwrap();

    // Create nodes (they get indexed automatically)
    executor
        .execute(
            Parser::new(
                r#"CREATE (n:Article {title: "Graph Database Systems", body: "graph database tutorial"})"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        )
        .unwrap();

    executor
        .execute(
            Parser::new(
                r#"CREATE (n:Article {title: "Relational Databases", body: "SQL and tables"})"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        )
        .unwrap();

    executor
        .execute(
            Parser::new(
                r#"CREATE (n:Article {title: "Machine Learning", body: "neural networks"})"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        )
        .unwrap();

    // Search using the procedure call
    let result = executor
        .execute(
            Parser::new(
                r#"CALL db.index.fulltext.search('article_idx', 'graph') YIELD node, score"#,
            )
            .unwrap()
            .parse()
            .unwrap(),
        )
        .unwrap();

    // Should return rows with "graph" in them (2 articles)
    assert!(!result.rows.is_empty());
    // Scores should be positive
    for row in &result.rows {
        if let Value::Float(score) = row.columns[1] {
            assert!(score > 0.0, "score should be positive");
        } else {
            panic!("second column should be a float score");
        }
    }
    // Columns should be ["node", "score"]
    assert_eq!(result.columns[0], "node");
    assert_eq!(result.columns[1], "score");
}

#[test]
fn test_procedure_call_fulltext_search_unknown_index() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    let result = executor.execute(
        Parser::new(r#"CALL db.index.fulltext.search('nonexistent', 'query') YIELD node, score"#)
            .unwrap()
            .parse()
            .unwrap(),
    );

    assert!(result.is_err());
}

#[test]
fn test_procedure_call_fulltext_search_returns_sorted_scores() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);

    executor
        .execute(
            Parser::new(r#"CREATE FULLTEXT INDEX idx FOR (n:Doc) ON (n.text)"#)
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap();

    // Document with "graph" appearing many times should rank higher
    executor
        .execute(
            Parser::new(r#"CREATE (n:Doc {text: "graph graph graph graph graph"})"#)
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap();

    executor
        .execute(
            Parser::new(r#"CREATE (n:Doc {text: "graph systems"})"#)
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap();

    let result = executor
        .execute(
            Parser::new(r#"CALL db.index.fulltext.search('idx', 'graph') YIELD node, score"#)
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap();

    assert_eq!(result.rows.len(), 2);
    // Scores should be in descending order
    if let (Value::Float(s1), Value::Float(s2)) = (
        result.rows[0].columns[1].clone(),
        result.rows[1].columns[1].clone(),
    ) {
        assert!(s1 >= s2, "results should be sorted by score descending");
    }
}

#[test]
fn test_procedure_yield_alias_where_match() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    fulltext_yield_fixture(&mut executor);

    let result = run_query(
        &mut executor,
        "CALL db.index.fulltext.search('articleIndex', 'rust') YIELD node AS article, score \
         WHERE score > 0.0 \
         MATCH (article)-[:AUTHORED_BY]->(author:Person) \
         RETURN article.title AS title, author.name AS author ORDER BY title",
    )
    .unwrap();
    assert_eq!(result.columns, vec!["title", "author"]);
    let rows: Vec<Vec<Value>> = result.rows.into_iter().map(|r| r.columns).collect();
    assert_eq!(
        rows,
        vec![
            vec![
                Value::String("Rust async guide".into()),
                Value::String("Alice".into())
            ],
            vec![
                Value::String("Rust basics".into()),
                Value::String("Bob".into())
            ],
        ]
    );

    // The original column name is not bound once aliased (bug/162: an error, not null).
    assert!(
        run_query(
            &mut executor,
            "CALL db.index.fulltext.search('articleIndex', 'rust') YIELD node AS article RETURN node",
        )
        .is_err()
    );
    // Unknown YIELD columns are rejected.
    assert!(
        run_query(
            &mut executor,
            "CALL db.index.fulltext.search('articleIndex', 'rust') YIELD nope RETURN nope",
        )
        .is_err()
    );
}

#[test]
fn test_procedure_yield_with_aggregation() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    fulltext_yield_fixture(&mut executor);

    let result = run_query(
        &mut executor,
        "CALL db.index.fulltext.search('articleIndex', 'rust') YIELD node \
         WITH count(node) AS hits WHERE hits > 1 RETURN hits",
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns, vec![Value::Int(2)]);
}

#[test]
fn test_procedure_yield_foreach() {
    let mut graph = Graph::new();
    let mut executor = Executor::new(&mut graph);
    fulltext_yield_fixture(&mut executor);

    let stmt = Parser::new(
        "CALL db.index.fulltext.search('articleIndex', 'rust') YIELD node \
         FOREACH (n IN [node] | SET n.topic = 'rust')",
    )
    .unwrap()
    .parse()
    .unwrap();
    assert!(!is_read_only(&stmt), "FOREACH after YIELD writes");
    executor.execute(stmt).unwrap();

    let result = run_query(
        &mut executor,
        "MATCH (a:Article) WHERE a.topic = 'rust' RETURN count(a)",
    )
    .unwrap();
    assert_eq!(result.rows[0].columns, vec![Value::Int(2)]);
}

#[test]
fn test_call_subquery_union_column_mismatch_and_explain() {
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:Person {name: 'Alice'})").unwrap();
    let mut executor = Executor::new(&mut graph);
    // UNION branches must agree on their columns.
    assert!(
        run_query(
            &mut executor,
            "CALL { MATCH (n:Person) RETURN n.name AS a UNION MATCH (n:Person) RETURN n.name AS b } RETURN a",
        )
        .is_err()
    );
    // Standalone CALL { } and shortestPath plans can be explained.
    for q in [
        "EXPLAIN CALL { MATCH (n:Person) RETURN count(n) AS c } RETURN c",
        "EXPLAIN MATCH p = shortestPath((a:Person)-[*]-(b:Person)) RETURN p",
        "EXPLAIN CALL db.labels() YIELD label AS l MATCH (n) RETURN l, n",
    ] {
        run_query(&mut executor, q).unwrap();
    }
}

#[test]
fn test_procedure_continuation_requires_yield_and_return() {
    // WHERE / MATCH need YIELD first.
    assert!(
        Parser::new("CALL db.labels() WHERE label = 'A' RETURN label")
            .unwrap()
            .parse()
            .is_err()
    );
    // MATCH after YIELD needs something to consume its rows.
    assert!(
        Parser::new("CALL db.labels() YIELD label MATCH (n)")
            .unwrap()
            .parse()
            .is_err()
    );
    // A bare YIELD … WHERE is a valid standalone call.
    assert!(
        Parser::new("CALL db.labels() YIELD label WHERE label = 'A'")
            .unwrap()
            .parse()
            .is_ok()
    );
}

#[test]
fn test_parse_procedure_call() {
    let stmt = Parser::new(r#"CALL db.index.fulltext.search('my_idx', 'query') YIELD node, score"#)
        .unwrap()
        .parse()
        .unwrap();

    if let Statement::ProcedureCall(pc) = stmt {
        assert_eq!(pc.procedure, "db.index.fulltext.search");
        assert_eq!(pc.arguments.len(), 2);
        assert_eq!(
            pc.yield_items
                .iter()
                .map(|i| i.column.as_str())
                .collect::<Vec<_>>(),
            vec!["node", "score"]
        );
    } else {
        panic!("expected ProcedureCall statement");
    }
}
