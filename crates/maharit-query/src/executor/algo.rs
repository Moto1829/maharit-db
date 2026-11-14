//! グラフアルゴリズムのプロシージャ（task172）。
//!
//! `maharit_core::algorithms` の関数は具体型の `Graph` を取るので、実行中のバックエンド
//! （`Graph` / `ConcurrentGraph`）から対象のノード・リレーションシップだけを写した
//! 一時的な `Graph`（トポロジーのみ、プロパティなし）を作って計算し、ノード ID を元に戻す。

use std::collections::{HashMap, HashSet};

use maharit_core::{
    Graph, NodeId, PageRankConfig, betweenness_centrality, closeness_centrality,
    connected_components, label_propagation, pagerank, strongly_connected_components,
};

use super::*;

/// プロシージャ名と YIELD できる列。`CALL` の分岐と docs の一覧に使う。
pub(super) const ALGORITHM_PROCEDURES: &[(&str, &[&str])] = &[
    ("db.pageRank", &["node", "score"]),
    ("db.betweennessCentrality", &["node", "score"]),
    ("db.closenessCentrality", &["node", "score"]),
    ("db.connectedComponents", &["node", "componentId"]),
    ("db.stronglyConnectedComponents", &["node", "componentId"]),
    ("db.labelPropagation", &["node", "communityId"]),
];

/// 一時グラフと、その ID → 元の ID の対応。
struct Snapshot {
    graph: Graph,
    original: HashMap<NodeId, NodeId>,
}

impl Snapshot {
    fn original_id(&self, id: NodeId) -> NodeId {
        self.original[&id]
    }
}

/// 引数のマップから読んだ設定。
struct Config {
    node_labels: Option<Vec<String>>,
    relationship_types: Option<Vec<String>>,
    map: HashMap<String, Value>,
}

impl Config {
    fn int(&self, key: &str) -> Result<Option<usize>, ExecuteError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Int(n)) if *n >= 1 => Ok(Some(*n as usize)),
            Some(other) => Err(ExecuteError::TypeError(format!(
                "{key} must be a positive integer, got {other}"
            ))),
        }
    }

    fn float(&self, key: &str) -> Result<Option<f64>, ExecuteError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Float(f)) => Ok(Some(*f)),
            Some(Value::Int(n)) => Ok(Some(*n as f64)),
            Some(other) => Err(ExecuteError::TypeError(format!(
                "{key} must be a number, got {other}"
            ))),
        }
    }
}

fn string_list(key: &str, value: &Value) -> Result<Vec<String>, ExecuteError> {
    let err = || ExecuteError::TypeError(format!("{key} must be a string or a list of strings"));
    match value {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                _ => Err(err()),
            })
            .collect(),
        _ => Err(err()),
    }
}

impl Executor<'_> {
    pub(super) fn algorithm_procedure(
        &self,
        name: &str,
        arguments: &[Expression],
    ) -> Result<(&'static [&'static str], Vec<Bindings>), ExecuteError> {
        let columns = ALGORITHM_PROCEDURES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .ok_or_else(|| ExecuteError::TypeError(format!("unknown procedure: {name}")))?;
        let config = self.algorithm_config(name, arguments)?;
        let snap = self.topology_snapshot(&config);
        let g = &snap.graph;

        let mut values: Vec<(NodeId, Value)> = match name {
            "db.pageRank" => {
                let mut pc = PageRankConfig::default();
                if let Some(n) = config.int("iterations")? {
                    pc.max_iterations = n;
                }
                if let Some(d) = config.float("dampingFactor")? {
                    if !(0.0..=1.0).contains(&d) {
                        return Err(ExecuteError::TypeError(
                            "dampingFactor must be between 0 and 1".to_string(),
                        ));
                    }
                    pc.damping = d;
                }
                if let Some(t) = config.float("tolerance")? {
                    pc.tolerance = t;
                }
                scores(&snap, pagerank(g, &pc).scores)
            }
            "db.betweennessCentrality" => scores(&snap, betweenness_centrality(g)),
            "db.closenessCentrality" => scores(&snap, closeness_centrality(g)),
            "db.connectedComponents" => components(&snap, connected_components(g)),
            "db.stronglyConnectedComponents" => components(&snap, strongly_connected_components(g)),
            "db.labelPropagation" => {
                let max = config.int("maxIterations")?.unwrap_or(10);
                // コミュニティ ID は代表ノードの一時 ID なので、元の ID に直す
                label_propagation(g, max)
                    .into_iter()
                    .map(|(id, community)| {
                        let community = snap.original.get(&community).copied().unwrap_or(community);
                        (snap.original_id(id), Value::Int(community as i64))
                    })
                    .collect()
            }
            _ => unreachable!("listed in ALGORITHM_PROCEDURES"),
        };
        values.sort_by_key(|(id, _)| *id);

        let rows = values
            .into_iter()
            .map(|(id, value)| {
                let mut b = Bindings::new();
                b.insert(Arc::from("node"), BindingValue::Node(id));
                b.insert(Arc::from(columns[1]), BindingValue::Scalar(value));
                b
            })
            .collect();
        Ok((columns, rows))
    }

    /// 引数は省略するか、設定のマップを 1 つ（`{nodeLabels: […], relationshipTypes: […], …}`）。
    fn algorithm_config(
        &self,
        name: &str,
        arguments: &[Expression],
    ) -> Result<Config, ExecuteError> {
        let map = match arguments {
            [] => HashMap::new(),
            [expr] => match self.evaluate_expression(expr, &Bindings::new())? {
                Value::Map(m) => m,
                Value::Null => HashMap::new(),
                other => {
                    return Err(ExecuteError::TypeError(format!(
                        "{name} takes a configuration map, got {other}"
                    )));
                }
            },
            _ => {
                return Err(ExecuteError::TypeError(format!(
                    "{name} takes at most one argument (a configuration map)"
                )));
            }
        };
        let allowed: &[&str] = match name {
            "db.pageRank" => &["iterations", "dampingFactor", "tolerance"],
            "db.labelPropagation" => &["maxIterations"],
            _ => &[],
        };
        for key in map.keys() {
            if key != "nodeLabels" && key != "relationshipTypes" && !allowed.contains(&key.as_str())
            {
                return Err(ExecuteError::TypeError(format!(
                    "{name}: unknown configuration key '{key}'"
                )));
            }
        }
        let list = |key: &str| -> Result<Option<Vec<String>>, ExecuteError> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => string_list(key, v).map(Some),
            }
        };
        Ok(Config {
            node_labels: list("nodeLabels")?,
            relationship_types: list("relationshipTypes")?,
            map,
        })
    }

    /// 対象のノード（`nodeLabels` のいずれかを持つもの。指定なしなら全部）と、その間の
    /// リレーションシップ（`relationshipTypes` のいずれか。指定なしなら全部）を一時グラフに写す。
    fn topology_snapshot(&self, config: &Config) -> Snapshot {
        let mut graph = Graph::new();
        let mut original = HashMap::new();
        let mut mapped: HashMap<NodeId, NodeId> = HashMap::new();

        let mut nodes = self.graph_ref().all_nodes();
        nodes.sort_by_key(|n| n.id);
        for node in nodes {
            if let Some(labels) = &config.node_labels
                && !labels.iter().any(|l| node.labels.contains(l))
            {
                continue;
            }
            let id = graph.create_node_with_labels(Vec::new());
            original.insert(id, node.id);
            mapped.insert(node.id, id);
        }

        let types: Option<HashSet<&str>> = config
            .relationship_types
            .as_ref()
            .map(|t| t.iter().map(String::as_str).collect());
        let mut edges = self.graph_ref().all_edges();
        edges.sort_by_key(|e| e.id);
        for edge in edges {
            if types
                .as_ref()
                .is_some_and(|t| !t.contains(edge.label.as_str()))
            {
                continue;
            }
            if let (Some(&from), Some(&to)) = (mapped.get(&edge.from), mapped.get(&edge.to)) {
                // 両端は今作ったノードなので失敗しない
                let _ = graph.create_edge(from, to, edge.label);
            }
        }
        Snapshot { graph, original }
    }
}

fn scores(snap: &Snapshot, scores: HashMap<NodeId, f64>) -> Vec<(NodeId, Value)> {
    scores
        .into_iter()
        .map(|(id, s)| (snap.original_id(id), Value::Float(s)))
        .collect()
}

/// 成分の ID は、成分に含まれる最小の（元の）ノード ID 順に 0, 1, 2, … と振る（結果を安定させるため）。
fn components(snap: &Snapshot, comps: Vec<HashSet<NodeId>>) -> Vec<(NodeId, Value)> {
    let mut comps: Vec<Vec<NodeId>> = comps
        .into_iter()
        .map(|c| {
            let mut ids: Vec<NodeId> = c.into_iter().map(|id| snap.original_id(id)).collect();
            ids.sort_unstable();
            ids
        })
        .collect();
    comps.sort_by_key(|c| c.first().copied());
    comps
        .into_iter()
        .enumerate()
        .flat_map(|(i, ids)| ids.into_iter().map(move |id| (id, Value::Int(i as i64))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn run(graph: &mut Graph, query: &str) -> Result<ResultSet, ExecuteError> {
        let stmt = Parser::new(query).unwrap().parse().unwrap();
        Executor::new(graph).execute(stmt)
    }

    fn sample() -> Graph {
        let mut g = Graph::new();
        run(
            &mut g,
            "CREATE (a:P {name: 'a'})-[:L]->(b:P {name: 'b'})-[:L]->(c:P {name: 'c'})-[:L]->(a), \
             (d:P {name: 'd'})-[:L]->(a), (x:Q {name: 'x'})-[:K]->(y:Q {name: 'y'})",
        )
        .unwrap();
        g
    }

    #[test]
    fn components_are_numbered_by_smallest_node() {
        let mut g = sample();
        let r = run(
            &mut g,
            "CALL db.connectedComponents() YIELD node, componentId \
             RETURN componentId, count(*) AS n ORDER BY componentId",
        )
        .unwrap();
        let rows: Vec<_> = r.rows.iter().map(|r| r.columns.clone()).collect();
        assert_eq!(
            rows,
            vec![
                vec![Value::Int(0), Value::Int(4)],
                vec![Value::Int(1), Value::Int(2)]
            ]
        );

        // 強連結成分は a→b→c→a の 3 つと、d・x・y がそれぞれ 1 つ
        let r = run(
            &mut g,
            "CALL db.stronglyConnectedComponents() YIELD componentId \
             RETURN count(DISTINCT componentId)",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(4));
    }

    #[test]
    fn filters_by_label_and_relationship_type() {
        let mut g = sample();
        let r = run(
            &mut g,
            "CALL db.connectedComponents({nodeLabels: ['Q']}) YIELD node RETURN count(node)",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(2));

        // K だけを辿ると P のノードはどれもつながらない
        let r = run(
            &mut g,
            "CALL db.connectedComponents({relationshipTypes: 'K'}) YIELD componentId \
             RETURN count(DISTINCT componentId)",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(5));
    }

    #[test]
    fn page_rank_and_write_back() {
        let mut g = sample();
        let r = run(
            &mut g,
            "CALL db.pageRank({iterations: 50, dampingFactor: 0.85}) YIELD node, score \
             RETURN node.name ORDER BY score DESC LIMIT 1",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::String("a".into()));

        let r = run(
            &mut g,
            "CALL db.pageRank() YIELD node, score SET node.rank = score",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(6));
        let r = run(&mut g, "MATCH (n) WHERE n.rank > 0 RETURN count(n)").unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(6));
    }

    #[test]
    fn centrality_and_label_propagation() {
        let mut g = sample();
        let r = run(
            &mut g,
            "CALL db.betweennessCentrality() YIELD node, score \
             RETURN node.name ORDER BY score DESC LIMIT 1",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::String("a".into()));
        let r = run(
            &mut g,
            "CALL db.closenessCentrality() YIELD score RETURN count(score)",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(6));
        let r = run(
            &mut g,
            "CALL db.labelPropagation({maxIterations: 5}) YIELD communityId \
             RETURN count(DISTINCT communityId)",
        )
        .unwrap();
        assert_eq!(r.rows[0].columns[0], Value::Int(2));
    }

    #[test]
    fn rejects_bad_configuration() {
        let mut g = sample();
        for q in [
            "CALL db.pageRank({foo: 1}) YIELD node RETURN node",
            "CALL db.pageRank({dampingFactor: 2}) YIELD node RETURN node",
            "CALL db.connectedComponents({iterations: 3}) YIELD node RETURN node",
            "CALL db.connectedComponents({nodeLabels: 1}) YIELD node RETURN node",
            "CALL db.pageRank(1) YIELD node RETURN node",
        ] {
            assert!(run(&mut g, q).is_err(), "{q}");
        }
    }
}
