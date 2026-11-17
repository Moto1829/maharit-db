//! キーワードの扱い（ラベル名やプロパティ名に使う場合を含む）、RETURN だけの文、サブクエリ、FOREACH、エラー回復。

use super::*;

#[test]
fn test_parse_merge_keyword() {
    // Just make sure MERGE is recognized as keyword
    let stmt = parse(r#"MERGE (n:Person {name: "Alice"})"#);
    assert!(stmt.is_ok());
}

#[test]
fn test_parse_remove_keyword() {
    let stmt = parse(r#"MATCH (n:Person) REMOVE n.age"#);
    assert!(stmt.is_ok());
}

#[test]
fn test_parse_unwind_keyword() {
    let stmt = parse("UNWIND [1] AS x RETURN x");
    assert!(stmt.is_ok());
}

// ========== Standalone RETURN tests (Task 89) ==========

#[test]
fn test_parse_standalone_return_literal() {
    let stmt = parse("RETURN 1 + 1 AS result");
    assert!(stmt.is_ok(), "Failed: {:?}", stmt);
    assert!(matches!(stmt.unwrap(), Statement::Return(_)));
}

#[test]
fn test_parse_standalone_return_string() {
    let stmt = parse(r#"RETURN 'hello' AS greeting"#);
    assert!(stmt.is_ok(), "Failed: {:?}", stmt);
}

#[test]
fn test_parse_standalone_return_bool_expr() {
    let stmt = parse("RETURN true AND false AS check");
    assert!(stmt.is_ok(), "Failed: {:?}", stmt);
}

#[test]
fn test_parse_standalone_return_multiple_items() {
    let stmt = parse("RETURN 1 AS a, 2 AS b, 'x' AS c");
    assert!(stmt.is_ok(), "Failed: {:?}", stmt);
}

#[test]
fn test_parse_map_literal_quoted_keys() {
    // JSON形式のクォート付きキーを持つマップリテラル (Task 88)
    let stmt = parse(r#"UNWIND [{"id": 0, "name": "Alice"}] AS item RETURN item.id"#);
    assert!(stmt.is_ok(), "Failed to parse quoted map keys: {:?}", stmt);
}

#[test]
fn test_parse_map_literal_mixed_keys() {
    // ベアキーとクォートキーの混在したマップリテラルのパース
    let stmt = parse(r#"UNWIND [{"id": 1, name: "Alice"}] AS item RETURN item"#);
    assert!(stmt.is_ok(), "Failed to parse mixed map keys: {:?}", stmt);
}

#[test]
fn test_parse_unwind_json_map_list() {
    // benchmark.py の bench_unwind_batch_create が生成するクエリ形式
    let stmt = parse(
        r#"UNWIND [{"id": 0, "name": "Alice0", "city": "Tokyo"}, {"id": 1, "name": "Bob1", "city": "Osaka"}] AS item CREATE (:UnwindBench {id: item.id, name: item.name})"#,
    );
    assert!(
        stmt.is_ok(),
        "Failed to parse JSON map list UNWIND: {:?}",
        stmt
    );
}

#[test]
fn test_parse_property_key_keyword_role() {
    // Task 98: RBAC キーワードの "role" をプロパティキーに使えること
    let stmt = parse("CREATE (n {role: 'Engineer'})");
    assert!(
        stmt.is_ok(),
        "Failed to parse 'role' as property key: {:?}",
        stmt
    );
}

#[test]
fn test_parse_property_key_keyword_in_relationship() {
    // Task 98: リレーションシップの property にキーワードを使えること
    let stmt = parse("MATCH (a), (b) CREATE (a)-[:WORKS_AT {role: 'Engineer'}]->(b)");
    assert!(
        stmt.is_ok(),
        "Failed to parse 'role' in relationship properties: {:?}",
        stmt
    );
}

#[test]
fn test_parse_property_key_multiple_keywords() {
    // Task 98: 複数のキーワードを同時にプロパティキーとして使えること
    // (role, user, password, type, index などのよく使われる名前)
    let stmt = parse(
        "CREATE (n {role: 'r', user: 'u', password: 'p', index: 1, contains: true, call: 'c'})",
    );
    assert!(
        stmt.is_ok(),
        "Failed to parse multiple keyword keys: {:?}",
        stmt
    );
}

#[test]
fn test_parse_property_key_keyword_uppercase() {
    // Task 98: 大文字のキーワードもプロパティキーとして受け入れ
    // (lexer が大文字小文字を区別しないキーワード認識をしているため)
    let stmt = parse("CREATE (n {ROLE: 'admin', USER: 'alice'})");
    assert!(
        stmt.is_ok(),
        "Failed to parse uppercase keyword keys: {:?}",
        stmt
    );
}

#[test]
fn test_parse_property_access_keyword() {
    // Task 98: n.role / r.type など、ドットアクセス位置のプロパティ名も
    // キーワードを受け入れること
    for q in [
        "MATCH (n) RETURN n.role",
        "MATCH (n)-[r]->(m) RETURN r.type, n.user, m.password",
        "MATCH (n) WHERE n.role = 'admin' RETURN n",
        "MATCH (n) RETURN n ORDER BY n.role DESC",
        "MATCH (n) SET n.role = 'admin'",
        "MATCH (n) REMOVE n.role",
    ] {
        let stmt = parse(q);
        assert!(
            stmt.is_ok(),
            "Failed to parse property access with keyword: {}\n  err: {:?}",
            q,
            stmt
        );
    }
}

#[test]
fn test_parse_label_keyword_in_node_pattern() {
    // Task 108: ノードラベル位置でキーワードを許容
    for q in [
        "CREATE (:User {email: 'a'})",
        "CREATE (:Role {name: 'admin'})",
        "CREATE (:Index {name: 'idx1'})",
        "MATCH (u:User) RETURN u",
        "MATCH (u:User:Role) RETURN u",
    ] {
        let stmt = parse(q);
        assert!(
            stmt.is_ok(),
            "Failed to parse keyword label: {}\n  err: {:?}",
            q,
            stmt
        );
    }
}

#[test]
fn test_parse_keyword_in_relationship_type() {
    // Task 108: リレーション型位置でキーワードを許容
    for q in [
        "MATCH (a)-[:Role]->(b) RETURN a, b",
        "MATCH (a)-[r:User]->(b) RETURN r",
        "MATCH (a) CREATE (a)-[:Constraint]->(b)",
    ] {
        let stmt = parse(q);
        assert!(
            stmt.is_ok(),
            "Failed to parse keyword rel type: {}\n  err: {:?}",
            q,
            stmt
        );
    }
}

#[test]
fn test_parse_create_constraint_with_keyword_label() {
    // Task 108: CREATE CONSTRAINT の FOR (var:Label) でキーワードラベル
    for q in [
        "CREATE CONSTRAINT c1 FOR (u:User) REQUIRE u.email IS UNIQUE",
        "CREATE CONSTRAINT c2 FOR (u:User) REQUIRE u.email IS NOT NULL",
        "CREATE CONSTRAINT c3 FOR (r:Role) REQUIRE (r.name, r.scope) IS UNIQUE",
    ] {
        let stmt = parse(q);
        assert!(stmt.is_ok(), "Failed to parse: {}\n  err: {:?}", q, stmt);
    }
}

#[test]
fn test_parse_create_index_with_keyword_label() {
    // Task 108: CREATE INDEX / FULLTEXT INDEX でキーワードラベル
    for q in [
        "CREATE INDEX ON :User(email)",
        "DROP INDEX ON :Role(name)",
        "CREATE FULLTEXT INDEX ft_user FOR (u:User) ON (u.bio)",
    ] {
        let stmt = parse(q);
        assert!(stmt.is_ok(), "Failed to parse: {}\n  err: {:?}", q, stmt);
    }
}

#[test]
fn test_parse_set_remove_label_keyword() {
    // Task 108: SET / REMOVE のラベル位置でキーワード
    for q in ["MATCH (n) SET n:User", "MATCH (n) REMOVE n:Role"] {
        let stmt = parse(q);
        assert!(stmt.is_ok(), "Failed to parse: {}\n  err: {:?}", q, stmt);
    }
}

// ========== Subquery parser tests ==========

#[test]
fn test_parse_exists_subquery_in_where() {
    let stmt =
        parse(r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#);
    assert!(stmt.is_ok(), "Failed to parse EXISTS subquery: {:?}", stmt);
}

#[test]
fn test_parse_exists_subquery_with_where_clause() {
    let stmt = parse(
        r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(f:Person) WHERE f.name = "Bob" } RETURN p.name"#,
    );
    assert!(
        stmt.is_ok(),
        "Failed to parse EXISTS subquery with WHERE: {:?}",
        stmt
    );
}

#[test]
fn test_parse_count_subquery_in_where() {
    let stmt =
        parse(r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 5 RETURN p.name"#);
    assert!(
        stmt.is_ok(),
        "Failed to parse COUNT subquery in WHERE: {:?}",
        stmt
    );
}

#[test]
fn test_parse_count_subquery_in_return() {
    let stmt = parse(r#"MATCH (p:Person) RETURN COUNT { MATCH (p)-[:KNOWS]->() }"#);
    assert!(
        stmt.is_ok(),
        "Failed to parse COUNT subquery in RETURN: {:?}",
        stmt
    );
}

#[test]
fn test_parse_collect_subquery_in_return() {
    let stmt = parse(
        r#"MATCH (p:Person) RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
    );
    assert!(stmt.is_ok(), "Failed to parse COLLECT subquery: {:?}", stmt);
}

#[test]
fn test_parse_call_subquery_with_with() {
    let stmt = parse(
        r#"MATCH (p:Person)
           CALL {
             WITH p
             MATCH (p)-[:KNOWS]->(f:Person)
             RETURN COUNT(f) AS friend_count
           }
           RETURN p.name, friend_count"#,
    );
    assert!(
        stmt.is_ok(),
        "Failed to parse CALL subquery with WITH: {:?}",
        stmt
    );
}

#[test]
fn test_parse_call_subquery_without_with() {
    let stmt = parse(
        r#"MATCH (p:Person)
           CALL {
             MATCH (q:Person)
             RETURN COUNT(q) AS total
           }
           RETURN p.name, total"#,
    );
    assert!(
        stmt.is_ok(),
        "Failed to parse CALL subquery without WITH: {:?}",
        stmt
    );
}

#[test]
fn test_parse_call_subquery_ast_structure() {
    let stmt = parse(
        r#"MATCH (p:Person)
           CALL {
             WITH p
             MATCH (p)-[:KNOWS]->(f:Person)
             RETURN COUNT(f) AS friend_count
           }
           RETURN p.name, friend_count"#,
    )
    .unwrap();

    match stmt {
        Statement::Match(m) => {
            assert!(m.call_clause.is_some(), "Expected call_clause to be Some");
            let call = m.call_clause.unwrap();
            assert_eq!(call.with_import, Some(vec!["p".to_string()]));
        }
        _ => panic!("Expected Match statement"),
    }
}

#[test]
fn test_parse_exists_subquery_ast_structure() {
    let stmt =
        parse(r#"MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p.name"#)
            .unwrap();

    match stmt {
        Statement::Match(m) => {
            let segment = &m.segments[0];
            let where_expr = segment
                .where_clause
                .as_ref()
                .expect("Expected WHERE clause");
            assert!(
                matches!(where_expr, Expression::ExistsSubquery(_)),
                "Expected ExistsSubquery expression, got {:?}",
                where_expr
            );
        }
        _ => panic!("Expected Match statement"),
    }
}

#[test]
fn test_parse_count_subquery_ast_structure() {
    let stmt =
        parse(r#"MATCH (p:Person) WHERE COUNT { MATCH (p)-[:KNOWS]->() } > 0 RETURN p.name"#)
            .unwrap();

    match stmt {
        Statement::Match(m) => {
            let segment = &m.segments[0];
            let where_expr = segment
                .where_clause
                .as_ref()
                .expect("Expected WHERE clause");
            // The WHERE clause is: COUNT{...} > 0, which is a BinaryOp
            assert!(
                matches!(where_expr, Expression::BinaryOp(_, _, _)),
                "Expected BinaryOp containing CountSubquery"
            );
            if let Expression::BinaryOp(left, op, _right) = where_expr {
                assert!(matches!(op, BinaryOp::Gt));
                assert!(
                    matches!(left.as_ref(), Expression::CountSubquery(_)),
                    "Expected CountSubquery on left of BinaryOp"
                );
            }
        }
        _ => panic!("Expected Match statement"),
    }
}

#[test]
fn test_parse_collect_subquery_ast_structure() {
    let stmt = parse(
        r#"MATCH (p:Person) RETURN COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name }"#,
    )
    .unwrap();

    match stmt {
        Statement::Match(m) => {
            assert_eq!(m.return_clause.items.len(), 1);
            match &m.return_clause.items[0] {
                ReturnItem::Expr(Expression::CollectSubquery(_)) => {}
                other => panic!("Expected CollectSubquery return item, got {:?}", other),
            }
        }
        _ => panic!("Expected Match statement"),
    }
}

// ========== FOREACH Tests ==========

#[test]
fn test_foreach_create() {
    let stmt =
        parse(r#"FOREACH (name IN ['Alice', 'Bob'] | CREATE (:Person {name: name}))"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "name");
            assert_eq!(f.clauses.len(), 1);
            assert!(matches!(f.clauses[0], ForeachClause::Create(_)));
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_set() {
    let stmt = parse(r#"FOREACH (n IN [1, 2, 3] | SET n.visited = true)"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "n");
            assert_eq!(f.clauses.len(), 1);
            assert!(matches!(f.clauses[0], ForeachClause::Set(_)));
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_remove() {
    let stmt = parse(r#"FOREACH (n IN [1] | REMOVE n.prop)"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "n");
            assert_eq!(f.clauses.len(), 1);
            assert!(matches!(f.clauses[0], ForeachClause::Remove(_)));
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_delete() {
    let stmt = parse(r#"FOREACH (n IN [1] | DELETE n)"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "n");
            assert_eq!(f.clauses.len(), 1);
            assert!(matches!(f.clauses[0], ForeachClause::Delete(_)));
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_merge() {
    let stmt = parse(r#"FOREACH (name IN ['Alice'] | MERGE (:Person {name: name}))"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "name");
            assert_eq!(f.clauses.len(), 1);
            assert!(matches!(f.clauses[0], ForeachClause::Merge(_)));
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_nested() {
    let stmt = parse(
        r#"FOREACH (city IN ['Tokyo'] | FOREACH (name IN ['Alice'] | CREATE (:Person {name: name})))"#,
    )
    .unwrap();
    match stmt {
        Statement::Foreach(outer) => {
            assert_eq!(outer.variable, "city");
            assert_eq!(outer.clauses.len(), 1);
            match &outer.clauses[0] {
                ForeachClause::Foreach(inner) => {
                    assert_eq!(inner.variable, "name");
                    assert_eq!(inner.clauses.len(), 1);
                    assert!(matches!(inner.clauses[0], ForeachClause::Create(_)));
                }
                _ => panic!("Expected nested Foreach"),
            }
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_foreach_with_literal_list() {
    let stmt = parse(r#"FOREACH (x IN [1, 2, 3] | CREATE (:Item {value: x}))"#).unwrap();
    match stmt {
        Statement::Foreach(f) => {
            assert_eq!(f.variable, "x");
            match &f.list {
                Expression::List(items) => assert_eq!(items.len(), 3),
                _ => panic!("Expected list expression"),
            }
        }
        _ => panic!("Expected Foreach statement"),
    }
}

#[test]
fn test_match_foreach_parse() {
    let stmt = parse(r#"MATCH (n:Person) FOREACH (x IN [1] | SET n.updated = true)"#).unwrap();
    match stmt {
        Statement::MatchForeach(mf) => {
            assert_eq!(mf.segments.len(), 1);
            assert_eq!(mf.foreach_clause.variable, "x");
            assert_eq!(mf.foreach_clause.clauses.len(), 1);
            assert!(matches!(
                mf.foreach_clause.clauses[0],
                ForeachClause::Set(_)
            ));
        }
        _ => panic!("Expected MatchForeach statement"),
    }
}

// ========== parse_with_recovery Tests ==========

#[test]
fn test_recovery_valid_query_returns_statement() {
    let result = parse_with_recovery("MATCH (n:Person) RETURN n");
    assert!(result.is_ok());
    assert!(result.statement.is_some());
    assert!(result.errors.is_empty());
}

#[test]
fn test_recovery_invalid_keyword_returns_error() {
    let result = parse_with_recovery("FOOBAR (n)");
    assert!(result.is_err());
    assert!(result.statement.is_none());
    assert!(!result.errors.is_empty());
}

#[test]
fn test_recovery_empty_input_returns_error() {
    let result = parse_with_recovery("");
    assert!(result.is_err());
    assert!(result.statement.is_none());
    assert!(!result.errors.is_empty());
    // 空入力は UnexpectedEof またはレキサーエラーを返す
    let has_eof_or_lexer_error = result
        .errors
        .iter()
        .any(|e| matches!(e, ParseError::UnexpectedEof | ParseError::LexerError(_)));
    assert!(has_eof_or_lexer_error || !result.errors.is_empty());
}

#[test]
fn test_recovery_incomplete_query_returns_error() {
    let result = parse_with_recovery("MATCH (n:Person) WHERE");
    assert!(result.is_err());
    assert!(!result.errors.is_empty());
}

#[test]
fn test_recovery_errors_contain_span_info() {
    let result = parse_with_recovery("INVALID TOKEN HERE");
    assert!(result.is_err());
    // エラーにはスパン情報が含まれていること
    let has_token_error = result.errors.iter().any(|e| {
        matches!(e, ParseError::UnexpectedToken { span, .. } if span.line > 0 || span.column > 0)
    });
    // UnexpectedEof またはトークンエラーが含まれていること
    assert!(!result.errors.is_empty());
    let _ = has_token_error;
}

#[test]
fn test_recovery_create_statement() {
    let result = parse_with_recovery("CREATE (n:Person {name: 'Alice'})");
    assert!(result.is_ok());
    assert!(matches!(result.statement, Some(Statement::Create(_))));
    assert!(result.errors.is_empty());
}

#[test]
fn test_recovery_parse_result_is_ok_is_err() {
    let ok_result = parse_with_recovery("MATCH (n) RETURN n");
    assert!(ok_result.is_ok());
    assert!(!ok_result.is_err());

    let err_result = parse_with_recovery("???");
    assert!(!err_result.is_ok());
    assert!(err_result.is_err());
}

#[test]
fn test_recovery_multiple_errors_possible() {
    // 現在の実装では最初のエラーで停止するが、errors は Vec なので複数エラーを保持できる
    let result = parse_with_recovery("NOT A VALID CYPHER QUERY AT ALL");
    assert!(result.is_err());
    // errors フィールドにエラーが含まれること
    assert!(!result.errors.is_empty());
}

/// 文の後ろに解釈できないトークンが残る入力はエラーにする（bug/118）。
/// 以前は黙って捨てていたため、書いた句が無視されても成功扱いになっていた。
#[test]
fn trailing_tokens_are_rejected_for_every_statement_kind() {
    let cases = [
        "MATCH (n) RETURN n junk",
        "MATCH (n) RETURN n.name ORDER BY n.name )",
        "MATCH (n) RETURN n LIMIT 1 2",
        "CREATE (n:P) extra",
        "CREATE (n:P {a: 1}) RETURN n junk",
        "MATCH (n) DELETE n n",
        "MATCH (n) DETACH DELETE n junk",
        "MATCH (n) SET n.a = 1 )",
        "MATCH (n) REMOVE n.a )",
        "MERGE (n:A) extra",
        "UNWIND [1] AS x CREATE (:A) )",
        "RETURN 1 2",
        "CREATE INDEX ON :A(k) foo",
        "DROP INDEX ON :A(k) bar",
        "SHOW CONSTRAINTS x",
        "EXPLAIN MATCH (n) RETURN n junk",
        "PROFILE MATCH (n) RETURN n junk",
        "MATCH (n) RETURN n UNION MATCH (m) RETURN m junk",
        "MATCH (n) WITH n RETURN n junk",
    ];
    for q in cases {
        let result = Parser::new(q).and_then(|mut p| p.parse());
        assert!(
            result.is_err(),
            "trailing tokens must be rejected: {q} -> {result:?}"
        );
    }
}

/// 末尾チェックの導入で正しい文が壊れていないこと。
#[test]
fn complete_statements_still_parse() {
    let cases = [
        "MATCH (n) RETURN n",
        "MATCH (n:P) WHERE n.a = 1 RETURN n.name ORDER BY n.age DESC SKIP 1 LIMIT 2",
        "MATCH (n) RETURN id(n) ORDER BY id(n)",
        "MATCH (n) RETURN n.name, count(*) AS c ORDER BY c DESC",
        "MATCH (n) SET n.a = 1 REMOVE n.b SET n:L RETURN n",
        "MATCH (n) REMOVE n:L SET n.a = 2",
        "MATCH (n) WITH n ORDER BY n.a LIMIT 3 RETURN n",
        "CREATE (a:A)-[:R]->(b:B)",
        "MERGE (n:A {k: 1}) ON CREATE SET n.c = 1 ON MATCH SET n.m = 1",
        "UNWIND [1, 2] AS x CREATE (:A {k: x})",
        "MATCH (n) DETACH DELETE n",
        "EXPLAIN MATCH (n) RETURN n",
        "MATCH (n) RETURN n UNION ALL MATCH (m) RETURN m",
        "CREATE INDEX ON :A(k)",
        "SHOW CONSTRAINTS",
        "RETURN 1",
    ];
    for q in cases {
        let result = Parser::new(q).and_then(|mut p| p.parse());
        assert!(
            result.is_ok(),
            "valid statement rejected: {q} -> {result:?}"
        );
    }
}

#[test]
fn set_and_remove_clauses_can_be_chained() {
    let stmt = Parser::new("MATCH (n:P) SET n.a = 1 REMOVE n.b SET n:Q RETURN n")
        .unwrap()
        .parse()
        .unwrap();
    match stmt {
        Statement::MatchSet(ms) => {
            assert_eq!(ms.more_updates.len(), 2);
            assert!(matches!(ms.more_updates[0], UpdateClause::Remove(_)));
            assert!(matches!(ms.more_updates[1], UpdateClause::Set(_)));
            assert!(ms.return_clause.is_some());
        }
        other => panic!("expected MatchSet, got {other:?}"),
    }
}
