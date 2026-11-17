//! 句の連結や演算子など、個別のバグ修正で足した回帰テスト。

use super::*;

#[test]
fn test_match_filters_inline_edge_properties() {
    // bug/155: MATCH のリレーションシップのインラインプロパティが無視されていた
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (a:A {n: 1})-[:K {w: 1}]->(b:B {n: 2})").unwrap();
    execute(
        &mut graph,
        "MATCH (b:B) CREATE (b)-[:K {w: 2}]->(:C {n: 3})",
    )
    .unwrap();
    execute(
        &mut graph,
        "MATCH (a:A) CREATE (a)-[:K {w: 2}]->(:B {n: 4})",
    )
    .unwrap();

    let r = execute(&mut graph, "MATCH ()-[r:K {w: 1}]->() RETURN r.w").unwrap();
    assert_eq!(r.row_count(), 1);
    let r = execute(
        &mut graph,
        "MATCH (x)-[r {w: 2}]->(y) RETURN y.n ORDER BY y.n",
    )
    .unwrap();
    assert_eq!(r.row_count(), 2);
    let r = execute(&mut graph, "MATCH (a:A)<-[r {w: 1}]-(x) RETURN x").unwrap();
    assert_eq!(r.row_count(), 0);
    // 可変長: 全てのホップが条件を満たす経路だけ
    let r = execute(&mut graph, "MATCH (a:A)-[*1..2 {w: 1}]->(x) RETURN x.n").unwrap();
    assert_eq!(r.row_count(), 1);
    let r = execute(&mut graph, "MATCH (a:A)-[*1..2]->(x) RETURN x.n").unwrap();
    assert_eq!(r.row_count(), 3);
    // パラメータ・存在しないキー
    let r = execute(&mut graph, "MATCH ()-[r {missing: 1}]->() RETURN r").unwrap();
    assert_eq!(r.row_count(), 0);
    let r = execute(
        &mut graph,
        "MATCH p = shortestPath((a:A)-[*..3 {w: 2}]->(c:C)) RETURN p",
    )
    .unwrap();
    assert_eq!(r.row_count(), 0);
}

#[test]
fn test_list_literal_index_slice_and_where_only_comprehension() {
    let mut graph = Graph::new();
    let r = execute(
        &mut graph,
        "RETURN [1, 2, 3][0], [1, 2, 3][-1], [1, 2, 3][9], [1, 2, 3, 4, 5][1..3], \
         [1, 2, 3, 4, 5][..3], [1, 2, 3, 4, 5][2..], [1, 2, 3, 4, 5][-2..], [1] + [2]",
    )
    .unwrap();
    let list = |v: &[i64]| Value::List(v.iter().map(|i| Value::Int(*i)).collect());
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::Int(1),
            Value::Int(3),
            Value::Null,
            list(&[2, 3]),
            list(&[1, 2, 3]),
            list(&[3, 4, 5]),
            list(&[4, 5]),
            list(&[1, 2]),
        ]
    );
    let r = execute(
        &mut graph,
        "RETURN [x IN [1, 2, 3, 4, 5] WHERE x % 2 = 0] AS e",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], list(&[2, 4]));
    assert!(
        Parser::new("RETURN [x IN [1, 2]]")
            .unwrap()
            .parse()
            .is_err()
    );
}

#[test]
fn test_relationship_values_support_property_access() {
    // bug/169: relationships(p) / collect(r) の要素から r.prop を参照できなかった
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (t:City {name: 'Tokyo'})-[:ROAD {distance: 350}]->(n:City {name: 'Nagoya'})\
         -[:ROAD {distance: 190}]->(o:City {name: 'Osaka'}), (t)-[:ROAD {distance: 600}]->(o)",
    )
    .unwrap();
    let r = execute(
        &mut graph,
        "MATCH p = (a:City {name: 'Tokyo'})-[:ROAD*1..5]->(b:City {name: 'Osaka'}) \
         RETURN reduce(total = 0, r IN relationships(p) | total + r.distance) AS km ORDER BY km",
    )
    .unwrap();
    let kms: Vec<Value> = r.rows.iter().map(|row| row.columns[0].clone()).collect();
    assert_eq!(kms, vec![Value::Int(540), Value::Int(600)]);
    let r = execute(
        &mut graph,
        "MATCH ()-[r:ROAD]->() WITH collect(r) AS rs \
         RETURN reduce(s = 0, x IN rs | s + x.distance), reduce(s = 0, x IN rs | s + x['distance'])",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(1140), Value::Int(1140)]);
    // 出力は従来どおり ID の整数
    let r = execute(
        &mut graph,
        "MATCH ()-[r:ROAD {distance: 350}]->() RETURN r, id(r)",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns[0].to_json(),
        r.rows[0].columns[1].to_json()
    );
}

#[test]
fn test_out_of_scope_variable_is_an_error() {
    // bug/162: WITH で引き継がなかった変数は null ではなくエラー
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (:Person {name: 'A', role: 'x'})-[:K]->(:Person {name: 'B'})",
    )
    .unwrap();
    for q in [
        "MATCH (n:Person) WITH n.role AS r RETURN n.role",
        "MATCH (n:Person) WITH n.role AS r RETURN n",
        "MATCH (n:Person) RETURN m.name",
    ] {
        let err = execute(&mut graph, q).unwrap_err();
        assert!(
            matches!(err, ExecuteError::UndefinedVariable(_)),
            "{q}: {err}"
        );
    }
    // OPTIONAL MATCH without a match still gives null
    let r = execute(
        &mut graph,
        "MATCH (n:Person {name: 'B'}) OPTIONAL MATCH (n)-[r:K]->(m) RETURN n.name, m.name, m, r",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::String("B".into()),
            Value::Null,
            Value::Null,
            Value::Null
        ]
    );
    let r = execute(
        &mut graph,
        "MATCH (n:Person) WITH n.role AS r RETURN r ORDER BY r",
    )
    .unwrap();
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_explain_reflects_property_indexes() {
    // bug/167: EXPLAIN showed NodeByLabelScan even with an index
    let mut graph = Graph::new();
    let mut ex = Executor::new(&mut graph);
    let mut plan = |q: &str| -> String {
        let r = ex
            .execute(Parser::new(q).unwrap().parse().unwrap())
            .unwrap();
        r.rows
            .iter()
            .map(|row| format!("{:?}", row.columns[0]))
            .collect::<Vec<_>>()
            .join("\n")
    };
    plan("UNWIND range(1, 20) AS i CREATE (:Person {name: toString(i), age: i})");
    assert!(plan("EXPLAIN MATCH (n:Person {name: '1'}) RETURN n").contains("NodeByLabelScan"));
    plan("CREATE INDEX ON :Person(name)");
    plan("CREATE INDEX ON :Person(age)");
    let p = plan("EXPLAIN MATCH (n:Person {name: '1'}) RETURN n");
    assert!(p.contains("IndexSeek") && p.contains(":Person.name"), "{p}");
    let p = plan("EXPLAIN MATCH (n:Person) WHERE n.name = '1' RETURN n");
    assert!(p.contains("IndexSeek"), "{p}");
    let p = plan("EXPLAIN MATCH (n:Person) WHERE n.age > 15 RETURN n");
    assert!(
        p.contains("IndexRangeScan") && p.contains(":Person.age"),
        "{p}"
    );
    let p = plan("PROFILE MATCH (n:Person {name: '1'}) RETURN n");
    assert!(p.contains("IndexSeek"), "{p}");
}

#[test]
fn test_fulltext_index_follows_set_remove_and_delete() {
    // bug/168: SET / DELETE が全文検索インデックスに反映されず、
    // 削除したノードや古い本文が検索に残っていた
    let mut graph = Graph::new();
    let mut ex = Executor::new(&mut graph);
    let mut run = |q: &str| {
        ex.execute(Parser::new(q).unwrap().parse().unwrap())
            .unwrap()
    };
    let hits = |r: ResultSet| r.rows[0].columns[0].clone();
    run("CREATE FULLTEXT INDEX ft FOR (a:Article) ON EACH [a.body]");
    run("CREATE (:Article {id: 1, body: 'rust graph'}), (:Article {id: 2, body: 'rust async'})");
    let count = "CALL db.index.fulltext.search('ft', 'rust') YIELD node, score RETURN count(node)";
    assert_eq!(hits(run(count)), Value::Int(2));
    run("MATCH (a:Article {id: 2}) SET a.body = 'nothing here'");
    assert_eq!(hits(run(count)), Value::Int(1));
    run("MATCH (a:Article {id: 2}) SET a.body = 'rust again'");
    assert_eq!(hits(run(count)), Value::Int(2));
    run("MATCH (a:Article {id: 2}) REMOVE a.body");
    assert_eq!(hits(run(count)), Value::Int(1));
    run("MATCH (a:Article {id: 1}) DETACH DELETE a");
    assert_eq!(hits(run(count)), Value::Int(0));
    run("CREATE (:Article {id: 3, body: 'rust'})");
    run("MATCH (a:Article {id: 3}) FOREACH (x IN [1] | DELETE a)");
    assert_eq!(hits(run(count)), Value::Int(0));
}

#[test]
fn test_dynamic_property_access() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (:User {name: 'a', email: 'e', created_at: 1})-[:R {w: 5}]->(:User {name: 'b'})",
    )
    .unwrap();
    let r = execute(
        &mut graph,
        "MATCH (u:User) WHERE all(f IN ['name', 'email', 'created_at'] WHERE exists(u[f])) \
         RETURN u.name, u['email'] AS e, u['missing'] AS m",
    )
    .unwrap();
    assert_eq!(r.row_count(), 1);
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::String("a".into()),
            Value::String("e".into()),
            Value::Null
        ]
    );
    let r = execute(&mut graph, "MATCH ()-[r:R]->() RETURN r['w'], r['x']").unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(5), Value::Null]);
    let r = execute(
        &mut graph,
        "MATCH (u:User) WITH collect(u) AS us RETURN us[0]['name'] IN ['a', 'b'] AS ok",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Bool(true));
}

#[test]
fn test_merge_relationship_binds_variable_on_create() {
    // bug/165: ON CREATE SET r.x で `undefined variable: r` になり、作成だけが残っていた
    let mut graph = Graph::new();
    execute(&mut graph, "CREATE (:P {name: 'A'}), (:P {name: 'B'})").unwrap();
    let q = "MATCH (a:P {name: 'A'}), (b:P {name: 'B'}) \
             MERGE (a)-[r:KNOWS]->(b) \
             ON CREATE SET r.since = 2024 ON MATCH SET r.seen = 2025 \
             RETURN r.since, r.seen";
    let r = execute(&mut graph, q).unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(2024), Value::Null]);
    let r = execute(&mut graph, q).unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(2024), Value::Int(2025)]);
    assert_eq!(graph.edge_count(), 1);
}

#[test]
fn test_xor_operator() {
    let mut graph = Graph::new();
    let r = execute(
        &mut graph,
        "RETURN true XOR false, true XOR true, false XOR true AND false, \
         true OR true XOR true, null XOR true",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(false),
            Value::Bool(true),
            Value::Null
        ]
    );
    execute(
        &mut graph,
        "CREATE (:P {a: true, b: false}), (:P {a: true, b: true})",
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (p:P) WHERE p.a XOR p.b RETURN p").unwrap();
    assert_eq!(r.row_count(), 1);
}

#[test]
fn test_pattern_comprehension() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (a:Person {name: 'Alice'})-[:KNOWS]->(:Person {name: 'Bob', age: 30}), \
         (a)-[:KNOWS]->(:Person {name: 'Carol', age: 20})",
    )
    .unwrap();
    let r = execute(
        &mut graph,
        "MATCH (p:Person {name: 'Alice'}) \
         RETURN [(p)-[:KNOWS]->(f) WHERE f.age > 25 | f.name] AS older, \
                size([(p)-[:KNOWS]->(f) | f]) AS n",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::List(vec![Value::String("Bob".into())]),
            Value::Int(2)
        ]
    );
    let r = execute(
        &mut graph,
        "MATCH (p:Person {name: 'Bob'}) RETURN [(p)-->(x) | x] AS l",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::List(vec![]));
    // 括弧で始まる普通のリストはそのまま
    let r = execute(&mut graph, "RETURN [(1 + 2), 3] AS l").unwrap();
    assert_eq!(
        r.rows[0].columns[0],
        Value::List(vec![Value::Int(3), Value::Int(3)])
    );
}

#[test]
fn test_chained_comparison_and_skip_limit_expressions() {
    let mut graph = Graph::new();
    execute(&mut graph, "UNWIND range(1, 10) AS i CREATE (:P {n: i})").unwrap();
    let r = execute(
        &mut graph,
        "MATCH (p:P) WHERE 3 <= p.n < 6 RETURN p.n ORDER BY p.n",
    )
    .unwrap();
    assert_eq!(r.row_count(), 3);
    let r = execute(
        &mut graph,
        "RETURN 1 < 2 < 3 AS a, 1 < 3 < 2 AS b, 2 = 2 = 2 AS c",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]
    );

    let stmt =
        Parser::new("MATCH (p:P) RETURN p.n ORDER BY p.n SKIP $size * ($page - 1) LIMIT $size")
            .unwrap()
            .parse()
            .unwrap();
    let params = HashMap::from([
        ("size".to_string(), Value::Int(3)),
        ("page".to_string(), Value::Int(2)),
    ]);
    let r = Executor::new(&mut graph)
        .execute_with_params(stmt, params)
        .unwrap();
    let ns: Vec<Value> = r.rows.iter().map(|row| row.columns[0].clone()).collect();
    assert_eq!(ns, vec![Value::Int(4), Value::Int(5), Value::Int(6)]);
    assert!(execute(&mut graph, "MATCH (p:P) RETURN p SKIP 2 - 3").is_err());
    assert!(execute(&mut graph, "MATCH (p:P) RETURN p LIMIT p.n").is_err());
}

#[test]
fn test_label_predicates_and_multiple_labels() {
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (:Person {name: 'A'})-[:KNOWS]->(:Company {name: 'B'}), (:Draft:Review {name: 'D'})",
    )
    .unwrap();
    let r = execute(
        &mut graph,
        "MATCH (n) WHERE n:Person OR n:Company RETURN n.name",
    )
    .unwrap();
    assert_eq!(r.row_count(), 2);
    let r = execute(&mut graph, "MATCH (n) WHERE n:Draft:Review RETURN n.name").unwrap();
    assert_eq!(r.row_count(), 1);
    execute(&mut graph, "MATCH (n {name: 'A'}) SET n:Admin:Moderator").unwrap();
    let r = execute(
        &mut graph,
        "MATCH (n {name: 'A'}) RETURN labels(n), n:Admin AS a",
    )
    .unwrap();
    let labels = |v: &[&str]| Value::List(v.iter().map(|l| Value::String(l.to_string())).collect());
    assert_eq!(
        r.rows[0].columns,
        vec![labels(&["Person", "Admin", "Moderator"]), Value::Bool(true)]
    );
    execute(&mut graph, "MATCH (n) WHERE n:Draft REMOVE n:Draft:Review").unwrap();
    let r = execute(&mut graph, "MATCH (n {name: 'D'}) RETURN labels(n)").unwrap();
    assert_eq!(r.rows[0].columns[0], labels(&[]));
    let r = execute(&mut graph, "MATCH ()-[r]->() RETURN r:KNOWS, r:OTHER").unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![Value::Bool(true), Value::Bool(false)]
    );
}

#[test]
fn test_math_functions() {
    let mut graph = Graph::new();
    let r = execute(
        &mut graph,
        "RETURN sin(0), cos(0), atan2(1.0, 1.0), exp(0), pow(2, 10), pow(4, 0.5), \
         degrees(pi()), radians(180), pow(null, 2), sin(null)",
    )
    .unwrap();
    let pi = std::f64::consts::PI;
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::Float(0.0),
            Value::Float(1.0),
            Value::Float(pi / 4.0),
            Value::Float(1.0),
            Value::Float(1024.0),
            Value::Float(2.0),
            Value::Float(180.0),
            Value::Float(pi),
            Value::Null,
            Value::Null,
        ]
    );
    // 式の中・WHERE でも使える
    execute(&mut graph, "CREATE (:L {x: 0, y: 0}), (:L {x: 3, y: 4})").unwrap();
    let r = execute(
        &mut graph,
        "MATCH (a:L), (b:L) WHERE a.x = 0 AND b.x = 3 \
         RETURN sqrt(pow(b.x - a.x, 2) + pow(b.y - a.y, 2)) AS d",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Float(5.0));
    assert!(execute(&mut graph, "RETURN sin('x')").is_err());
}

#[test]
fn test_modulo_operator() {
    let mut graph = Graph::new();
    let r = execute(
        &mut graph,
        "RETURN 7 % 3 AS a, -7 % 3 AS b, 7.5 % 2 AS c, 2 + 7 % 3 * 2 AS d",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            Value::Int(1),
            Value::Int(-1),
            Value::Float(1.5),
            Value::Int(4)
        ]
    );
    let r = execute(
        &mut graph,
        "RETURN all(x IN [2, 4, 6] WHERE x % 2 = 0) AS even",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Bool(true));
    let err = execute(&mut graph, "RETURN 1 % 0").unwrap_err().to_string();
    assert!(err.contains("division by zero"), "{err}");
    let err = execute(&mut graph, "RETURN (-9223372036854775807 - 1) % -1")
        .unwrap_err()
        .to_string();
    assert!(err.contains("integer overflow"), "{err}");
}

#[test]
fn test_integer_division_by_zero_and_overflow_are_errors() {
    // bug/161: 0 除算・オーバーフローで panic していた（panic=abort でサーバーごと落ちる）
    let mut graph = Graph::new();
    for q in [
        "RETURN 1 / 0",
        "RETURN 9223372036854775807 + 1",
        "RETURN -9223372036854775807 - 2",
        "RETURN 4611686018427387904 * 2",
        "RETURN (-9223372036854775807 - 1) / -1",
        "RETURN -(-9223372036854775807 - 1)",
        "RETURN abs(-9223372036854775807 - 1)",
        "UNWIND [0] AS z RETURN 10 / z",
    ] {
        let err = execute(&mut graph, q).unwrap_err().to_string();
        assert!(
            err.contains("division by zero") || err.contains("integer overflow"),
            "{q}: {err}"
        );
    }
    // 浮動小数点の 0 除算は Infinity
    let r = execute(&mut graph, "RETURN 1.0 / 0 AS x").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Float(f64::INFINITY));
    let r = execute(
        &mut graph,
        "RETURN 7 / 2 AS x, 9223372036854775806 + 1 AS y",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(3), Value::Int(i64::MAX)]);
}

#[test]
fn test_list_property_values() {
    // graph/146: リストをプロパティに保存し、IN・size()・添字・UNWIND・比較に使える
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (:A {t: 'a', tags: ['x', 'y'], nums: [1, 2.5, null]}), (:A {t: 'b', tags: ['y']})",
    )
    .unwrap();
    let s = |v: &str| Value::String(v.into());

    let r = execute(
        &mut graph,
        "MATCH (n:A) WHERE 'x' IN n.tags RETURN n.t, size(n.tags), n.tags[1], n.nums",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns,
        vec![
            s("a"),
            Value::Int(2),
            s("y"),
            Value::List(vec![Value::Int(1), Value::Float(2.5), Value::Null])
        ]
    );

    let r = execute(
        &mut graph,
        "MATCH (n:A) UNWIND n.tags AS tag RETURN count(tag)",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(3));

    // パターンのプロパティ・WHERE の等号でリスト同士を比べる
    let r = execute(&mut graph, "MATCH (n:A {tags: ['y']}) RETURN n.t").unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].columns[0], s("b"));
    let r = execute(
        &mut graph,
        "MATCH (n:A) WHERE n.tags = ['x', 'y'] RETURN n.t",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], s("a"));

    // SET でリストを更新
    let r = execute(
        &mut graph,
        "MATCH (n:A {t: 'b'}) SET n.tags = n.tags + ['z'] RETURN n.tags",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::List(vec![s("y"), s("z")]));

    // リストのリスト・マップを含むリストは保存できない
    assert!(execute(&mut graph, "CREATE (:B {v: [[1]]})").is_err());
    assert!(execute(&mut graph, "CREATE (:B {v: [{a: 1}]})").is_err());
}

#[test]
fn test_detach_delete_counts_removed_edges() {
    // bug/142: DETACH DELETE で一緒に消えたエッジを deleted_edges に数える
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (a:P {n: 1})-[:K]->(b:P {n: 2})-[:K]->(c:P {n: 3})",
    )
    .unwrap();
    execute(&mut graph, "MATCH (b:P {n: 2}) CREATE (b)-[:SELF]->(b)").unwrap();

    let r = execute(&mut graph, "MATCH (n:P {n: 2}) DETACH DELETE n").unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(1), Value::Int(3)]);

    // 2 ノードを同時に消すとき、両者の間のエッジは 1 回だけ数える
    execute(
        &mut graph,
        "MATCH (a:P {n: 1}), (c:P {n: 3}) CREATE (a)-[:K]->(c)",
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (n:P) DETACH DELETE n").unwrap();
    assert_eq!(r.rows[0].columns, vec![Value::Int(2), Value::Int(1)]);
    assert_eq!(graph.edge_count(), 0);
}

#[test]
fn test_clause_chaining_unwind_match_with() {
    // task163: UNWIND / MATCH / WITH を自由な順序でつなぐ
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (:A {t: 'A', tags: 'x,y'}), (:A {t: 'B', tags: 'y'}), (:P {id: 1}), (:P {id: 3}), (:P {id: 9})",
    )
    .unwrap();
    let s = |v: &str| Value::String(v.into());

    // MATCH の後の UNWIND
    let r = execute(
        &mut graph,
        "MATCH (a:A) UNWIND split(a.tags, ',') AS tag RETURN a.t, tag ORDER BY a.t, tag",
    )
    .unwrap();
    assert_eq!(column(&r, 1), vec![s("x"), s("y"), s("y")]);

    // UNWIND の後の MATCH（UNWIND の変数で絞り込む）
    let r = execute(
        &mut graph,
        "UNWIND [1, 2, 3] AS id MATCH (n:P) WHERE n.id = id RETURN n.id ORDER BY n.id",
    )
    .unwrap();
    assert_eq!(column(&r, 0), vec![Value::Int(1), Value::Int(3)]);

    // UNWIND の連続、null は 0 行
    let r = execute(
        &mut graph,
        "UNWIND [[1, 2], [3]] AS sub UNWIND sub AS n RETURN collect(n)",
    )
    .unwrap();
    assert_eq!(
        r.rows[0].columns[0],
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    let r = execute(
        &mut graph,
        "UNWIND null AS x UNWIND [1] AS y RETURN count(*)",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(0));

    // WITH の後の UNWIND、WITH … WHERE の後の MATCH
    let r = execute(
        &mut graph,
        "MATCH (a:A) WITH collect(a.t) AS ts UNWIND ts AS t WITH t WHERE t <> 'A' RETURN t",
    )
    .unwrap();
    assert_eq!(column(&r, 0), vec![s("B")]);
    let r = execute(
        &mut graph,
        "MATCH (a:A) WITH a WHERE a.t = 'A' MATCH (p:P {id: 9}) RETURN a.t, p.id",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns, vec![s("A"), Value::Int(9)]);

    // ノードのリストを UNWIND すると、ノードとしてパターンに使える
    let r = execute(
        &mut graph,
        "MATCH (a:A) WITH collect(a) AS xs UNWIND xs AS x MATCH (x:A) RETURN count(x)",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(2));

    // トップレベルの MATCH … MATCH、WITH・OPTIONAL MATCH で始まる文
    let r = execute(&mut graph, "MATCH (a:A) MATCH (p:P) RETURN count(*)").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(6));
    let r = execute(&mut graph, "WITH 1 AS x RETURN x + 1").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(2));
    let r = execute(&mut graph, "OPTIONAL MATCH (z:Nope) RETURN z").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Null);

    // UNWIND … MATCH … SET（書き込み）
    execute(
        &mut graph,
        "UNWIND [1, 9] AS i MATCH (p:P {id: i}) SET p.hit = true",
    )
    .unwrap();
    let r = execute(&mut graph, "MATCH (p:P) WHERE p.hit = true RETURN count(p)").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(2));
}

#[test]
fn test_null_property_value_removes_property() {
    // bug/174: null を設定するとプロパティは消える。null のプロパティは作らない
    let mut graph = Graph::new();
    // keys() の順序は決まっていないので、リストは並べ替えて比べる
    let keys = |g: &mut Graph, q: &str| match execute(g, q).unwrap().rows[0].columns[0].clone() {
        Value::List(mut items) => {
            items.sort_by_key(|v| v.to_string());
            Value::List(items)
        }
        other => other,
    };
    let list = |v: &[&str]| Value::List(v.iter().map(|s| Value::String(s.to_string())).collect());

    execute(
        &mut graph,
        "CREATE (n:Q {a: 1, b: null, c: 3})-[:R {w: null, x: 1}]->(:Q)",
    )
    .unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH (n:Q {a: 1}) RETURN keys(n)"),
        list(&["a", "c"])
    );
    assert_eq!(
        keys(&mut graph, "MATCH ()-[r:R]->() RETURN keys(r)"),
        list(&["x"])
    );

    execute(&mut graph, "MATCH (n:Q {a: 1}) SET n.a = null").unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH (n:Q {c: 3}) RETURN keys(n)"),
        list(&["c"])
    );

    execute(&mut graph, "MATCH (n:Q {c: 3}) SET n += {c: null, d: 4}").unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH (n:Q {d: 4}) RETURN keys(n)"),
        list(&["d"])
    );

    execute(&mut graph, "MATCH (n:Q {d: 4}) SET n = {d: null, e: 5}").unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH (n:Q {e: 5}) RETURN keys(n)"),
        list(&["e"])
    );

    execute(&mut graph, "MATCH ()-[r:R]->() SET r.x = null").unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH ()-[r:R]->() RETURN size(keys(r))"),
        Value::Int(0)
    );

    // プロパティインデックスからも消える
    execute(&mut graph, "CREATE INDEX ON :Q(e)").unwrap();
    execute(&mut graph, "MATCH (n:Q {e: 5}) SET n.e = null").unwrap();
    assert_eq!(
        keys(&mut graph, "MATCH (n:Q {e: 5}) RETURN count(n)"),
        Value::Int(0)
    );
}

#[test]
fn test_write_clause_chaining() {
    // task171: 書き込み句の後にも句を続けられる
    let mut graph = Graph::new();
    let count = |g: &mut Graph, q: &str| execute(g, q).unwrap().rows[0].columns[0].clone();

    // CREATE … CREATE、件数は 5 列で返す
    let r = execute(&mut graph, "CREATE (:Q {n: 1}) CREATE (:Q {n: 2})").unwrap();
    assert_eq!(r.columns[0], "created_nodes");
    assert_eq!(r.rows[0].columns[0], Value::Int(2));

    // MATCH … CREATE … SET … RETURN
    let r = execute(
        &mut graph,
        "MATCH (a:Q {n: 1}) CREATE (c:Q {n: 3}) SET c.from = a.n RETURN c.from",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(1));

    // CREATE … WITH … MATCH: 作ったノードが後続の MATCH に見える
    assert_eq!(
        count(
            &mut graph,
            "CREATE (x:Q {n: 4}) WITH x MATCH (q:Q) RETURN count(q)"
        ),
        Value::Int(4)
    );

    // UNWIND … MATCH … DETACH DELETE
    let r = execute(
        &mut graph,
        "UNWIND [1, 2] AS i MATCH (n:Q {n: i}) DETACH DELETE n",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[2], Value::Int(2));
    assert_eq!(
        count(&mut graph, "MATCH (q:Q) RETURN count(q)"),
        Value::Int(2)
    );

    // UNWIND … MERGE: 同じ値は 1 回だけ作る
    assert_eq!(
        count(
            &mut graph,
            "UNWIND [7, 8, 7] AS i MERGE (m:M {i: i}) RETURN count(m)"
        ),
        Value::Int(3)
    );
    assert_eq!(
        count(&mut graph, "MATCH (m:M) RETURN count(m)"),
        Value::Int(2)
    );

    // MATCH … WITH … MERGE（WITH を無視しない）
    execute(
        &mut graph,
        "MATCH (m:M) WITH m WHERE m.i = 7 MERGE (m)-[:SELF]->(m)",
    )
    .unwrap();
    assert_eq!(
        count(&mut graph, "MATCH ()-[r:SELF]->() RETURN count(r)"),
        Value::Int(1)
    );

    // MERGE … MERGE … MERGE
    let r = execute(
        &mut graph,
        "MERGE (a:Z {k: 1}) MERGE (b:Z {k: 2}) MERGE (a)-[:R]->(b)",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[..2], [Value::Int(2), Value::Int(1)]);

    // SET … CREATE … REMOVE
    execute(
        &mut graph,
        "MATCH (q:Q) SET q.t = 1 CREATE (:Log {n: q.n}) REMOVE q.t",
    )
    .unwrap();
    assert_eq!(
        count(&mut graph, "MATCH (l:Log) RETURN count(l)"),
        Value::Int(2)
    );
    assert_eq!(
        count(
            &mut graph,
            "MATCH (q:Q) WHERE q.t IS NOT NULL RETURN count(q)"
        ),
        Value::Int(0)
    );

    // 句の列にならない誤りは元のエラーのまま
    assert!(
        Parser::new("MATCH (n) RETURN n junk")
            .unwrap()
            .parse()
            .is_err()
    );
    assert!(Parser::new("CREATE (a) SET").unwrap().parse().is_err());
}

#[test]
fn test_multiple_relationship_types() {
    // task164: `[:A|B]` はいずれかの型に一致する
    let mut graph = Graph::new();
    execute(
        &mut graph,
        "CREATE (a:C {n: 'a'})-[:ROAD]->(b:C {n: 'b'})-[:FERRY]->(c:C {n: 'c'})-[:AIR]->(d:C {n: 'd'})",
    )
    .unwrap();

    let r = execute(
        &mut graph,
        "MATCH (x)-[r:ROAD|FERRY]->(y) RETURN type(r) AS t ORDER BY t",
    )
    .unwrap();
    let types: Vec<_> = r.rows.iter().map(|row| row.columns[0].clone()).collect();
    assert_eq!(
        types,
        vec![Value::String("FERRY".into()), Value::String("ROAD".into())]
    );

    // 旧来の `[:A|:B]` も同じ意味
    let r = execute(&mut graph, "MATCH ()-[r:ROAD|:AIR]->() RETURN count(r)").unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(2));

    // 可変長: ROAD/FERRY だけで a から c まで行ける、d へは AIR が要るので行けない
    let r = execute(
        &mut graph,
        "MATCH p = (a:C {n: 'a'})-[:ROAD|FERRY*1..5]->(b) RETURN b.n ORDER BY b.n",
    )
    .unwrap();
    let ends: Vec<_> = r.rows.iter().map(|row| row.columns[0].clone()).collect();
    assert_eq!(
        ends,
        vec![Value::String("b".into()), Value::String("c".into())]
    );

    // shortestPath
    let r = execute(
        &mut graph,
        "MATCH p = shortestPath((a:C {n: 'a'})-[:ROAD|FERRY*]->(c:C {n: 'c'})) RETURN length(p)",
    )
    .unwrap();
    assert_eq!(r.rows[0].columns[0], Value::Int(2));
    let r = execute(
        &mut graph,
        "MATCH p = shortestPath((a:C {n: 'a'})-[:ROAD|FERRY*]->(d:C {n: 'd'})) RETURN length(p)",
    )
    .unwrap();
    assert!(r.rows.is_empty());
}

#[test]
fn test_create_and_merge_reject_multiple_relationship_types() {
    // task164: 作るリレーションシップの型は 1 つに決まっていなければならない
    let mut graph = Graph::new();
    let err = execute(&mut graph, "CREATE (a)-[:A|B]->(b)").unwrap_err();
    assert!(matches!(err, ExecuteError::TypeError(_)), "{err:?}");
    execute(&mut graph, "CREATE (:X), (:Y)").unwrap();
    let err = execute(&mut graph, "MATCH (x:X), (y:Y) MERGE (x)-[:A|B]->(y)").unwrap_err();
    assert!(matches!(err, ExecuteError::TypeError(_)), "{err:?}");
    assert_eq!(graph.edge_count(), 0);
    assert_eq!(graph.node_count(), 2);
}

#[test]
fn test_foreach_delete_nodes() {
    let mut graph = Graph::new();
    // Create some nodes with no edges so DELETE works
    execute(&mut graph, r#"CREATE (:Temp)"#).unwrap();
    execute(&mut graph, r#"CREATE (:Temp)"#).unwrap();

    let count_before = execute(&mut graph, "MATCH (n:Temp) RETURN n").unwrap();
    assert_eq!(count_before.row_count(), 2);

    // FOREACH DELETE using MATCH+FOREACH
    execute(
        &mut graph,
        r#"MATCH (n:Temp) FOREACH (x IN [1] | DETACH DELETE n)"#,
    )
    .unwrap();

    let count_after = execute(&mut graph, "MATCH (n:Temp) RETURN n").unwrap();
    assert_eq!(count_after.row_count(), 0);
}

#[test]
fn test_match_foreach_set() {
    let mut graph = Graph::new();
    execute(&mut graph, r#"CREATE (:Node {active: true})"#).unwrap();
    execute(&mut graph, r#"CREATE (:Node {active: true})"#).unwrap();

    execute(
        &mut graph,
        r#"MATCH (n:Node) FOREACH (x IN [1] | SET n.visited = true)"#,
    )
    .unwrap();

    let result = execute(
        &mut graph,
        r#"MATCH (n:Node) WHERE n.visited = true RETURN n"#,
    )
    .unwrap();
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_foreach_merge() {
    let mut graph = Graph::new();

    // FOREACH with MERGE - should create if not exists
    execute(
        &mut graph,
        r#"FOREACH (name IN ['Alice', 'Alice', 'Bob'] | MERGE (:Person {name: name}))"#,
    )
    .unwrap();

    // Alice should only exist once (MERGE deduplication)
    let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.row_count(), 2);
}

#[test]
fn test_foreach_multiple_clauses() {
    let mut graph = Graph::new();

    // FOREACH with multiple update clauses
    execute(
        &mut graph,
        r#"FOREACH (x IN [1] | CREATE (:A) CREATE (:B))"#,
    )
    .unwrap();

    let a = execute(&mut graph, "MATCH (n:A) RETURN n").unwrap();
    let b = execute(&mut graph, "MATCH (n:B) RETURN n").unwrap();
    assert_eq!(a.row_count(), 1);
    assert_eq!(b.row_count(), 1);
}

#[test]
fn test_foreach_empty_list() {
    let mut graph = Graph::new();

    // FOREACH with empty list should create no nodes
    execute(
        &mut graph,
        r#"FOREACH (name IN [] | CREATE (:Person {name: name}))"#,
    )
    .unwrap();

    let result = execute(&mut graph, "MATCH (n:Person) RETURN n").unwrap();
    assert_eq!(result.row_count(), 0);
}

// ---- Task 39: Predicate Functions ----
