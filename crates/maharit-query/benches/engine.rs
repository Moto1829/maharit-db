//! エンジン単体のベンチマーク（TCP・JSON・WAL を通さない）。
//!
//! サーバーと同じ `ConcurrentGraph` の上でクエリを実行し、パーサーと
//! エグゼキュータだけの時間を測る。サーバーは AST をキャッシュするので、
//! `read` / `write` ではパース済みの文を複製して実行する。
//!
//! ```bash
//! cargo bench -p maharit-query --bench engine                 # 全部
//! cargo bench -p maharit-query --bench engine -- read/hop     # 名前で絞る
//! cargo bench -p maharit-query --bench engine -- --save-baseline before
//! cargo bench -p maharit-query --bench engine -- --baseline before
//! ```
//!
//! データ: Person 10,000（id / name / age / city）、City 100、
//! KNOWS は 1 人あたり 5 本、LIVES_IN は 1 人 1 本。
//! Hub は LIKES 10,000 本と RARE 10 本を持つノード 1 つ。

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use maharit_core::{
    ConcurrentGraph, ConstraintManager, FulltextManager, GraphBackend, PropertyIndex, PropertyValue,
};
use maharit_query::ast::Statement;
use maharit_query::{AccessPolicy, Executor, Parser, ResultSet};

const PERSONS: u64 = 10_000;
const CITIES: u64 = 100;
const KNOWS_PER_PERSON: u64 = 5;
const HUB_LIKES: u64 = 10_000;
const HUB_RARE: u64 = 10;

/// グラフと、サーバーが共有している 3 つのマネージャ。
struct Fixture {
    graph: ConcurrentGraph,
    constraints: ConstraintManager,
    fulltext: FulltextManager,
    index: PropertyIndex,
}

impl Fixture {
    fn empty() -> Self {
        Self {
            graph: ConcurrentGraph::new(),
            constraints: ConstraintManager::new(),
            fulltext: FulltextManager::new(),
            index: PropertyIndex::new(),
        }
    }

    /// ベンチ用のデータを入れたグラフ。`Person(id)` に索引を張る。
    fn populated() -> Self {
        let mut fx = Self::empty();
        let g = &mut fx.graph;
        // 実行のたびに同じグラフになるよう、乱数は固定の線形合同法で作る
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };

        let cities: Vec<_> = (0..CITIES)
            .map(|i| {
                let id = GraphBackend::create_node_with_labels(g, vec!["City".to_string()]);
                GraphBackend::set_node_property(
                    g,
                    id,
                    "name",
                    PropertyValue::String(format!("City{i}")),
                );
                id
            })
            .collect();

        let persons: Vec<_> = (0..PERSONS)
            .map(|i| {
                let id = GraphBackend::create_node_with_labels(g, vec!["Person".to_string()]);
                let city = i % CITIES;
                GraphBackend::set_node_property(g, id, "id", PropertyValue::Int(i as i64));
                GraphBackend::set_node_property(
                    g,
                    id,
                    "name",
                    PropertyValue::String(format!("Person{i}")),
                );
                GraphBackend::set_node_property(
                    g,
                    id,
                    "age",
                    PropertyValue::Int(20 + (i % 50) as i64),
                );
                GraphBackend::set_node_property(
                    g,
                    id,
                    "city",
                    PropertyValue::String(format!("City{city}")),
                );
                GraphBackend::create_edge(g, id, cities[city as usize], "LIVES_IN".to_string())
                    .unwrap();
                id
            })
            .collect();

        for &from in &persons {
            for _ in 0..KNOWS_PER_PERSON {
                let to = persons[next(PERSONS) as usize];
                GraphBackend::create_edge(g, from, to, "KNOWS".to_string()).unwrap();
            }
        }

        let hub = GraphBackend::create_node_with_labels(g, vec!["Hub".to_string()]);
        for i in 0..HUB_LIKES {
            let to = persons[(i % PERSONS) as usize];
            GraphBackend::create_edge(g, hub, to, "LIKES".to_string()).unwrap();
        }
        for i in 0..HUB_RARE {
            GraphBackend::create_edge(g, hub, persons[i as usize], "RARE".to_string()).unwrap();
        }

        fx.write(&parse("CREATE INDEX FOR (n:Person) ON (n.id)"));
        fx
    }

    /// 読み取りの実行（サーバーの読み取り経路と同じ作り方）。
    fn read(&self, stmt: &Statement, policy: Option<&AccessPolicy>) -> ResultSet {
        // SAFETY: このベンチは 1 スレッドで、実行中にグラフを書き換えない。
        let mut executor = unsafe {
            Executor::new_concurrent_shared(
                &self.graph,
                &self.constraints,
                &self.fulltext,
                &self.index,
            )
        }
        .with_access_policy(policy.cloned());
        executor
            .execute_with_params(stmt.clone(), HashMap::new())
            .expect("query failed")
    }

    /// 書き込みの実行（サーバーの書き込み経路と同じ作り方。WAL と undo の記録は含まない）。
    fn write(&mut self, stmt: &Statement) -> ResultSet {
        Executor::new_with_backend_exclusive(
            &mut self.graph,
            &mut self.constraints,
            &mut self.fulltext,
            &mut self.index,
        )
        .execute_with_params(stmt.clone(), HashMap::new())
        .expect("query failed")
    }
}

fn parse(query: &str) -> Statement {
    Parser::new(query)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("parse error in {query:?}: {e}"))
}

/// サーバーが応答を組み立てるのと同じ形（行ごとに 列名 → JSON 値）にして直列化する。
fn to_response_json(result: &ResultSet) -> Vec<u8> {
    let rows: Vec<HashMap<String, serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .zip(row.columns.iter())
                .map(|(col, val)| (col.clone(), val.to_json()))
                .collect()
        })
        .collect();
    serde_json::to_vec(&rows).unwrap()
}

const PARSE_QUERIES: &[(&str, &str)] = &[
    (
        "point_lookup",
        "MATCH (n:Person) WHERE n.id = 4242 RETURN n.name",
    ),
    (
        "two_hop_aggregate",
        "MATCH (a:Person {id: 42})-[:KNOWS]->(b:Person)-[:LIVES_IN]->(c:City) \
         WHERE b.age > 30 AND c.name <> 'City1' \
         WITH c, count(b) AS friends, avg(b.age) AS age \
         WHERE friends > 1 \
         RETURN c.name AS city, friends, age ORDER BY friends DESC, city LIMIT 10",
    ),
];

/// 読み取りクエリ。`(名前, クエリ, 期待する行数)`。行数はクエリが意図どおり
/// 動いていること（空振りを測っていないこと）を実行前に確かめるために持つ。
const READ_QUERIES: &[(&str, &str, usize)] = &[
    // ── スキャン ──
    ("scan/count", "MATCH (n:Person) RETURN count(n)", 1),
    (
        "scan/return_property",
        "MATCH (n:Person) RETURN n.name",
        10_000,
    ),
    ("scan/return_node", "MATCH (n:Person) RETURN n", 10_000),
    // ── 絞り込み ──
    (
        "filter/eq_indexed",
        "MATCH (n:Person) WHERE n.id = 4242 RETURN n.name",
        1,
    ),
    (
        "filter/range_indexed",
        "MATCH (n:Person) WHERE n.id < 100 RETURN n.name",
        100,
    ),
    (
        "filter/eq_unindexed",
        "MATCH (n:Person) WHERE n.city = 'City7' RETURN n.id",
        100,
    ),
    (
        "filter/expression",
        "MATCH (n:Person) WHERE n.age + 1 > 60 RETURN n.id",
        2_000,
    ),
    // ── 1 点から辿る ──
    (
        "hop/point_1",
        "MATCH (a:Person {id: 42})-[:KNOWS]->(b) RETURN b.name",
        5,
    ),
    (
        "hop/point_2",
        "MATCH (a:Person {id: 42})-[:KNOWS]->(b)-[:KNOWS]->(c) RETURN c.name",
        25,
    ),
    (
        "hop/point_3",
        "MATCH (a:Person {id: 42})-[:KNOWS]->(b)-[:KNOWS]->(c)-[:KNOWS]->(d) RETURN d.name",
        125,
    ),
    // ── 全体を辿る ──
    (
        "hop/all_1",
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN count(*)",
        1,
    ),
    (
        "hop/all_2",
        "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person) RETURN count(*)",
        1,
    ),
    (
        "hop/all_1_where_both_ends",
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.age > b.age + 30 RETURN count(*)",
        1,
    ),
    // ── 種別の違うエッジが多いノード（10,010 本のうち 10 本だけが対象）──
    (
        "hop/rare_type_on_hub",
        "MATCH (h:Hub)-[:RARE]->(x) RETURN x.id",
        10,
    ),
    // ── 右端のほうが絞り込める（City 1 件 ← Person 100 件）──
    (
        "hop/selective_end",
        "MATCH (p:Person)-[:LIVES_IN]->(c:City {name: 'City7'}) RETURN p.id",
        100,
    ),
    // ── 可変長・最短経路 ──
    (
        "path/var_length_1_3",
        "MATCH (a:Person {id: 42})-[:KNOWS*1..3]->(b) RETURN b.id",
        155,
    ),
    (
        "path/shortest",
        "MATCH p = shortestPath((a:Person {id: 42})-[:KNOWS*..6]->(b:Person {id: 4242})) \
         RETURN length(p)",
        1,
    ),
    // ── 集計・並べ替え ──
    (
        "agg/group_by",
        "MATCH (p:Person) RETURN p.city, count(*), avg(p.age)",
        100,
    ),
    (
        "agg/distinct",
        "MATCH (p:Person) RETURN DISTINCT p.city",
        100,
    ),
    (
        "sort/order_by_limit",
        "MATCH (p:Person) RETURN p.name ORDER BY p.age DESC LIMIT 10",
        10,
    ),
    (
        "sort/order_by_all",
        "MATCH (p:Person) RETURN p.name ORDER BY p.name",
        10_000,
    ),
];

/// ACL を付けたセッションでも測るクエリ（索引を使うもの・辿るもの）。
const ACL_QUERIES: &[&str] = &["filter/eq_indexed", "hop/all_1", "scan/return_property"];

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for (name, query) in PARSE_QUERIES {
        group.bench_function(*name, |b| b.iter(|| parse(black_box(query))));
    }
    group.finish();
}

fn bench_read(c: &mut Criterion) {
    let fx = Fixture::populated();
    let mut group = c.benchmark_group("read");
    for (name, query, expected_rows) in READ_QUERIES {
        let stmt = parse(query);
        let rows = fx.read(&stmt, None).rows.len();
        assert_eq!(rows, *expected_rows, "{name}: unexpected row count");
        group.bench_function(*name, |b| b.iter(|| fx.read(black_box(&stmt), None)));
    }
    group.finish();

    // ACL のあるセッション。規則はベンチのデータに当たらない（:Secret は存在しない）
    // ので結果は同じで、差は ACL のラッパーと索引を使わないことによるもの。
    let mut policy = AccessPolicy::default();
    policy.read.labels.insert("Secret".to_string(), true);
    let mut group = c.benchmark_group("read_acl");
    for (name, query, expected_rows) in READ_QUERIES {
        if !ACL_QUERIES.contains(name) {
            continue;
        }
        let stmt = parse(query);
        let rows = fx.read(&stmt, Some(&policy)).rows.len();
        assert_eq!(rows, *expected_rows, "{name} (acl): unexpected row count");
        group.bench_function(*name, |b| {
            b.iter(|| fx.read(black_box(&stmt), Some(&policy)))
        });
    }
    group.finish();

    // 応答の組み立て（10,000 行を JSON にする）。クエリの実行は含まない。
    let mut group = c.benchmark_group("response");
    for (name, query) in [
        (
            "json_10k_properties",
            "MATCH (n:Person) RETURN n.id, n.name, n.age",
        ),
        ("json_10k_nodes", "MATCH (n:Person) RETURN n"),
    ] {
        let result = fx.read(&parse(query), None);
        assert_eq!(result.rows.len(), PERSONS as usize);
        group.bench_function(name, |b| b.iter(|| to_response_json(black_box(&result))));
    }
    group.finish();
}

fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("write");

    // 空のグラフに対する作成。グラフは反復ごとに作り直す（作り直しは計測外）。
    for (name, setup, query) in [
        (
            "create_node",
            None,
            "CREATE (:Item {id: 1, name: 'item', price: 100})",
        ),
        (
            "create_node_indexed_unique",
            Some([
                "CREATE INDEX FOR (n:Item) ON (n.name)",
                "CREATE CONSTRAINT item_id FOR (n:Item) REQUIRE n.id IS UNIQUE",
            ]),
            "CREATE (:Item {id: 1, name: 'item', price: 100})",
        ),
        (
            "create_path",
            None,
            "CREATE (:Item {id: 1})-[:PART_OF {qty: 2}]->(:Item {id: 2})",
        ),
        (
            "unwind_create_1000",
            None,
            "UNWIND range(1, 1000) AS i CREATE (:Item {id: i, name: 'item'})",
        ),
    ] {
        let stmt = parse(query);
        let setup: Vec<Statement> = setup.iter().flatten().map(|q| parse(q)).collect();
        group.bench_function(name, |b| {
            b.iter_batched_ref(
                || {
                    let mut fx = Fixture::empty();
                    for s in &setup {
                        fx.write(s);
                    }
                    fx
                },
                |fx| fx.write(black_box(&stmt)),
                BatchSize::SmallInput,
            )
        });
    }

    // 10,000 ノードのグラフに対する更新。どれも繰り返しても状態が増えない文。
    let mut fx = Fixture::populated();
    for (name, query) in [
        ("set_property", "MATCH (n:Person {id: 42}) SET n.age = 33"),
        (
            "set_indexed_property_all",
            "MATCH (n:Person) SET n.id = n.id",
        ),
        ("merge_existing", "MERGE (n:Person {id: 42})"),
        (
            "create_delete_edge",
            "MATCH (a:Person {id: 1}), (b:Person {id: 2}) CREATE (a)-[r:TMP]->(b) DELETE r",
        ),
    ] {
        let stmt = parse(query);
        fx.write(&stmt);
        group.bench_function(name, |b| b.iter(|| fx.write(black_box(&stmt))));
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(30);
    targets = bench_parse, bench_read, bench_write
}
criterion_main!(benches);
