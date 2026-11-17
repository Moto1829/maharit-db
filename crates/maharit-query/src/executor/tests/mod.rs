//! executor.rs のテスト。分野ごとのファイルに分け、共通のヘルパーはここに置く。

use super::*;
use crate::parser::Parser;

mod clauses;
mod fulltext;
mod functions;
mod index_misc;
mod params_subquery;
mod predicates_labels;
mod read_basic;
mod schema;
mod temporal;
mod write;

fn execute(graph: &mut Graph, query: &str) -> Result<ResultSet, ExecuteError> {
    let stmt = Parser::new(query).unwrap().parse().unwrap();
    Executor::new(graph).execute(stmt)
}

fn execute_with(executor: &mut Executor<'_>, query: &str) -> Result<ResultSet, ExecuteError> {
    let stmt = Parser::new(query).unwrap().parse().unwrap();
    executor.execute(stmt)
}

// ────────────────────────────────────────────────────────────────
// WHERE 述語のパターン押し下げ（フィルタプッシュダウン）
// ────────────────────────────────────────────────────────────────

/// 3 都市の Person を作るヘルパー。
fn seed_cities(graph: &mut Graph) {
    execute(
        graph,
        "CREATE (:Person {name: 'a', city: 'Tokyo', age: 40})",
    )
    .unwrap();
    execute(
        graph,
        "CREATE (:Person {name: 'b', city: 'Tokyo', age: 20})",
    )
    .unwrap();
    execute(
        graph,
        "CREATE (:Person {name: 'c', city: 'Osaka', age: 35})",
    )
    .unwrap();
    // 都市プロパティを持たないノード（欠損プロパティの扱いを確認）
    execute(graph, "CREATE (:Person {name: 'd', age: 50})").unwrap();
}

fn count_of(rs: &ResultSet) -> i64 {
    match &rs.rows[0].columns[0] {
        Value::Int(n) => *n,
        other => panic!("expected Int count, got {:?}", other),
    }
}

fn execute_with_constraints(
    graph: &mut Graph,
    queries: &[&str],
) -> Vec<Result<ResultSet, ExecuteError>> {
    let mut executor = Executor::new(graph);
    queries
        .iter()
        .map(|q| {
            let stmt = Parser::new(q).unwrap().parse().unwrap();
            executor.execute(stmt)
        })
        .collect()
}

/// Fulltext fixture for the YIELD-continuation tests: two Rust articles
/// (one by Alice, one by Bob) and one unrelated article.
fn fulltext_yield_fixture(executor: &mut Executor) {
    for q in [
        "CREATE FULLTEXT INDEX articleIndex FOR (n:Article) ON (n.title)",
        "CREATE (:Article {title: 'Rust async guide'})",
        "CREATE (:Article {title: 'Rust basics'})",
        "CREATE (:Article {title: 'Cooking pasta'})",
        "CREATE (:Person {name: 'Alice'})",
        "CREATE (:Person {name: 'Bob'})",
        "MATCH (a:Article {title: 'Rust async guide'}), (p:Person {name: 'Alice'}) CREATE (a)-[:AUTHORED_BY]->(p)",
        "MATCH (a:Article {title: 'Rust basics'}), (p:Person {name: 'Bob'}) CREATE (a)-[:AUTHORED_BY]->(p)",
    ] {
        executor
            .execute(Parser::new(q).unwrap().parse().unwrap())
            .unwrap();
    }
}

fn run_query(executor: &mut Executor, q: &str) -> Result<ResultSet, ExecuteError> {
    executor.execute(Parser::new(q).unwrap().parse().unwrap())
}

fn run_with(
    graph: &mut Graph,
    q: &str,
    params: &[(&str, Value)],
) -> Result<ResultSet, ExecuteError> {
    let stmt = Parser::new(q).unwrap().parse().unwrap();
    let params = params
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    Executor::new(graph).execute_with_params(stmt, params)
}

fn map_value(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn column(r: &ResultSet, i: usize) -> Vec<Value> {
    r.rows.iter().map(|row| row.columns[i].clone()).collect()
}

fn seed_people(graph: &mut Graph) {
    for (name, age) in [("c", 3), ("a", 1), ("b", 2)] {
        execute(
            graph,
            &format!("CREATE (:P {{name: '{name}', age: {age}}})"),
        )
        .unwrap();
    }
}

fn first_column(result: &ResultSet) -> Vec<Value> {
    result.rows.iter().map(|r| r.columns[0].clone()).collect()
}

fn strings(values: &[&str]) -> Vec<Value> {
    values
        .iter()
        .map(|v| Value::String(v.to_string()))
        .collect()
}
