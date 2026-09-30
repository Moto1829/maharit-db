//! Cypher conformance table (task121-4).
//!
//! Each case runs a standard Cypher query against a small fixed graph and
//! compares the result with the expected rows. Cases the engine does not
//! support yet are listed in [`KNOWN_UNSUPPORTED`]: they must still fail, so
//! when one starts passing the test tells you to move it into [`CASES`].

use maharit_core::Graph;
use serde_json::{Value as Json, json};

use crate::executor::Executor;
use crate::parser::Parser;

/// (alice:Person {name:'Alice', age:30}) -[:KNOWS {since:2010}]-> (bob:Person {name:'Bob', age:25})
/// (bob) -[:KNOWS {since:2015}]-> (carol:Person {name:'Carol', age:35})
/// (alice) -[:WORKS_AT]-> (acme:Company {name:'Acme'}) <-[:WORKS_AT]- (carol)
const FIXTURE: &[&str] = &[
    "CREATE (:Person {name: 'Alice', age: 30})",
    "CREATE (:Person {name: 'Bob', age: 25})",
    "CREATE (:Person {name: 'Carol', age: 35})",
    "CREATE (:Company {name: 'Acme'})",
    "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) CREATE (a)-[:KNOWS {since: 2010}]->(b)",
    "MATCH (b:Person {name: 'Bob'}), (c:Person {name: 'Carol'}) CREATE (b)-[:KNOWS {since: 2015}]->(c)",
    "MATCH (a:Person {name: 'Alice'}), (c:Company {name: 'Acme'}) CREATE (a)-[:WORKS_AT]->(c)",
    "MATCH (p:Person {name: 'Carol'}), (c:Company {name: 'Acme'}) CREATE (p)-[:WORKS_AT]->(c)",
];

/// How to compare result rows.
#[derive(Clone, Copy)]
enum Order {
    /// Row order is not significant.
    Any,
    /// Row order must match (query has ORDER BY).
    Exact,
}
use Order::{Any, Exact};

struct Case {
    query: &'static str,
    order: Order,
    rows: fn() -> Json,
}

macro_rules! case {
    ($q:expr, $order:expr, $rows:tt) => {
        Case {
            query: $q,
            order: $order,
            rows: || json!($rows),
        }
    };
}

fn cases() -> Vec<Case> {
    vec![
        // ── MATCH basics ────────────────────────────────────────────────
        case!(
            "MATCH (n:Person) RETURN n.name",
            Any,
            [["Alice"], ["Bob"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.age > 28 RETURN n.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.age >= 25 AND n.age <= 30 RETURN n.name",
            Any,
            [["Alice"], ["Bob"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.name = 'Alice' OR n.name = 'Carol' RETURN n.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.name IN ['Alice', 'Bob'] RETURN n.name",
            Any,
            [["Alice"], ["Bob"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.name STARTS WITH 'C' RETURN n.name",
            Any,
            [["Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE n.email IS NULL RETURN count(n)",
            Any,
            [[3]]
        ),
        case!(
            "MATCH (a:Person {name: 'Alice'}), (c:Company) RETURN a.name, c.name",
            Any,
            [["Alice", "Acme"]]
        ),
        // ── Relationship patterns ───────────────────────────────────────
        case!(
            "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
            Any,
            [["Alice", "Bob"], ["Bob", "Carol"]]
        ),
        case!(
            "MATCH (c:Company)<-[:WORKS_AT]-(p) RETURN p.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (a:Person {name: 'Bob'})-[:KNOWS]-(b) RETURN b.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (a:Person {name: 'Alice'})-[:KNOWS*1..2]->(b) RETURN b.name",
            Any,
            [["Bob"], ["Carol"]]
        ),
        case!(
            "MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE r.since > 2012 RETURN a.name",
            Any,
            [["Bob"]]
        ),
        case!(
            "MATCH (:Person {name: 'Alice'})-[r]->(:Company) RETURN type(r)",
            Any,
            [["WORKS_AT"]]
        ),
        // ── Anonymous nodes (bug/119) ───────────────────────────────────
        case!(
            "MATCH (:Person)-[r:KNOWS]->(:Person) RETURN r.since",
            Any,
            [[2010], [2015]]
        ),
        case!(
            "MATCH (a:Person {name: 'Alice'})-[:KNOWS]->(:Person) RETURN a.name",
            Any,
            [["Alice"]]
        ),
        case!(
            "MATCH (:Person {name: 'Alice'})-[:KNOWS]->(b) RETURN b.name",
            Any,
            [["Bob"]]
        ),
        case!(
            "MATCH (a)-[:WORKS_AT]->(:Company) RETURN a.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (a)-[:KNOWS]->()-[:KNOWS]->(c) RETURN a.name, c.name",
            Any,
            [["Alice", "Carol"]]
        ),
        case!("MATCH ()-[r:KNOWS]->() RETURN count(r)", Any, [[2]]),
        case!(
            "MATCH (a)-[:KNOWS]->(b)-[:KNOWS]->(c) RETURN a.name, b.name, c.name",
            Any,
            [["Alice", "Bob", "Carol"]]
        ),
        // ── OPTIONAL MATCH / WITH / UNWIND ──────────────────────────────
        case!(
            "MATCH (p:Person) OPTIONAL MATCH (p)-[:WORKS_AT]->(c) RETURN p.name, c.name",
            Any,
            [["Alice", "Acme"], ["Bob", null], ["Carol", "Acme"]]
        ),
        case!(
            "MATCH (n:Person) WITH n WHERE n.age < 35 RETURN n.name",
            Any,
            [["Alice"], ["Bob"]]
        ),
        case!("UNWIND [1, 2, 3] AS x RETURN x", Exact, [[1], [2], [3]]),
        // ── Aggregation / ordering ──────────────────────────────────────
        case!("MATCH (n:Person) RETURN count(n)", Any, [[3]]),
        case!("MATCH (n:Person) RETURN avg(n.age)", Any, [[30.0]]),
        case!(
            "MATCH (p:Person)-[:WORKS_AT]->(c:Company) RETURN c.name, count(p) AS n",
            Any,
            [["Acme", 2]]
        ),
        case!(
            "MATCH (p:Person)-[:WORKS_AT]->(c) RETURN DISTINCT c.name",
            Any,
            [["Acme"]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.name ORDER BY n.age DESC",
            Exact,
            [["Carol"], ["Alice"], ["Bob"]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.name ORDER BY n.name SKIP 1 LIMIT 1",
            Exact,
            [["Bob"]]
        ),
        // ── Expressions / functions ─────────────────────────────────────
        case!(
            "MATCH (n:Person {name: 'Alice'}) RETURN n.age + 1",
            Any,
            [[31]]
        ),
        case!("RETURN 'a' + 'b'", Any, [["ab"]]),
        case!(
            "MATCH (n:Person) RETURN n.name, CASE WHEN n.age > 28 THEN 'senior' ELSE 'junior' END AS g",
            Any,
            [["Alice", "senior"], ["Bob", "junior"], ["Carol", "senior"]]
        ),
        case!("MATCH (n:Company) RETURN labels(n)", Any, [[["Company"]]]),
        case!(
            "MATCH (n:Person) WHERE toLower(n.name) = 'alice' RETURN n.name",
            Any,
            [["Alice"]]
        ),
        case!(
            "MATCH (n:Person) WHERE size(n.name) > 3 RETURN n.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        // ── Pattern predicates / subqueries ─────────────────────────────
        case!(
            "MATCH (n:Person) WHERE (n)-[:WORKS_AT]->() RETURN n.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE NOT (n)-[:WORKS_AT]->(:Company) RETURN n.name",
            Any,
            [["Bob"]]
        ),
        case!(
            "MATCH (n:Person) WHERE EXISTS { (n)-[:KNOWS]->() } RETURN n.name",
            Any,
            [["Alice"], ["Bob"]]
        ),
        // ── Paths ───────────────────────────────────────────────────────
        case!(
            "MATCH p = (a:Person {name: 'Alice'})-[:KNOWS*]->(b:Person {name: 'Carol'}) RETURN length(p)",
            Any,
            [[2]]
        ),
        case!(
            "MATCH p = (:Person {name: 'Alice'})-[:KNOWS]->()-[:KNOWS]->(c) RETURN length(p), c.name",
            Any,
            [[2, "Carol"]]
        ),
        case!(
            "MATCH p = (a:Person {name: 'Bob'})-[:KNOWS]->(b) RETURN size(nodes(p))",
            Any,
            [[2]]
        ),
        // ── More expressions ────────────────────────────────────────────
        case!(
            "MATCH (n:Person) WHERE n.age IS NOT NULL RETURN count(n)",
            Any,
            [[3]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.name + ' (' + toString(n.age) + ')' AS label ORDER BY label",
            Exact,
            [["Alice (30)"], ["Bob (25)"], ["Carol (35)"]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.name ORDER BY n.age + 0",
            Exact,
            [["Bob"], ["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.age * 2 AS d ORDER BY d DESC LIMIT 1",
            Exact,
            [[70]]
        ),
        case!(
            "MATCH (n:Person) RETURN toUpper(n.name) = 'BOB' AS isBob, n.name ORDER BY n.name",
            Exact,
            [[false, "Alice"], [true, "Bob"], [false, "Carol"]]
        ),
        case!(
            "MATCH (n:Person) WHERE EXISTS { MATCH (n)-[:WORKS_AT]->(c:Company) WHERE c.name = 'Acme' } RETURN n.name",
            Any,
            [["Alice"], ["Carol"]]
        ),
        case!(
            "MATCH (n:Person) RETURN n.name, COUNT { (n)-[:KNOWS]-() } AS degree ORDER BY n.name",
            Exact,
            [["Alice", 1], ["Bob", 2], ["Carol", 1]]
        ),
    ]
}

/// Queries that are standard Cypher but not supported yet. Each must still
/// fail; move it to `cases()` once it works.
const KNOWN_UNSUPPORTED: &[&str] = &[
    "MATCH p = shortestPath((a:Person {name: 'Alice'})-[:KNOWS*]-(b:Person {name: 'Carol'})) RETURN length(p)",
    "MATCH (n:Person) WHERE n.age % 2 = 0 RETURN n.name",
    "MATCH (n:Person) RETURN count(n) + 1",
    "CREATE CONSTRAINT ON (n:Person) ASSERT n.name IS UNIQUE",
    "CALL db.labels() YIELD label AS l RETURN l",
];

fn fixture_graph() -> Graph {
    let mut graph = Graph::new();
    for q in FIXTURE {
        let stmt = Parser::new(q).unwrap().parse().unwrap();
        Executor::new(&mut graph).execute(stmt).unwrap();
    }
    graph
}

fn run(graph: &mut Graph, query: &str) -> Result<Vec<Json>, String> {
    let stmt = Parser::new(query)
        .and_then(|mut p| p.parse())
        .map_err(|e| format!("parse error: {e}"))?;
    let result = Executor::new(graph)
        .execute(stmt)
        .map_err(|e| format!("execution error: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|r| Json::Array(r.columns.iter().map(|v| v.to_json()).collect()))
        .collect())
}

fn canonical(rows: &[Json], order: Order) -> Vec<String> {
    let mut v: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    if matches!(order, Any) {
        v.sort();
    }
    v
}

#[test]
fn cypher_conformance_table() {
    let mut failures = Vec::new();
    for case in cases() {
        let mut graph = fixture_graph();
        let expected: Vec<Json> = (case.rows)().as_array().cloned().unwrap_or_default();
        match run(&mut graph, case.query) {
            Ok(actual) => {
                if canonical(&actual, case.order) != canonical(&expected, case.order) {
                    failures.push(format!(
                        "{}\n    expected {}\n    actual   {}",
                        case.query,
                        Json::Array(expected),
                        Json::Array(actual)
                    ));
                }
            }
            Err(e) => failures.push(format!("{}\n    {e}", case.query)),
        }
    }
    assert!(
        failures.is_empty(),
        "{} conformance case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn known_unsupported_queries_still_fail() {
    for query in KNOWN_UNSUPPORTED {
        let mut graph = fixture_graph();
        assert!(
            run(&mut graph, query).is_err(),
            "now supported — move it from KNOWN_UNSUPPORTED into cases(): {query}"
        );
    }
}
