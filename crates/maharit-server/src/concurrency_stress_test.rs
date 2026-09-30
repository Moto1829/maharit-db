//! Concurrency stress test with invariant checks (task121-2).
//!
//! Many writer clients (random writes, transactions, partially failing
//! statements — the same workload as the replication diff test) run against
//! one server while reader clients issue random read queries. Afterwards the
//! server's graph and indexes must be internally consistent:
//!
//! * reads never fail while writes are in flight,
//! * no dangling relationships; adjacency sets agree with the edges,
//! * the label index agrees with a full scan,
//! * the property index agrees with a full scan,
//! * the UNIQUE constraint on `:U(id)` holds,
//! * the server still answers.
//!
//! Reproducible from `MAHARIT_STRESS_SEED` (default 7); size with
//! `MAHARIT_STRESS_OPS` (per client, default 150).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use maharit_core::{ConcurrentGraph, GraphBackend, PropertyValue};
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};

use crate::replication_diff_test::{KEYS, Rng, index_mismatches, request, run_client};
use crate::tcp_server::{ServerConfig, TcpServer};

const WRITERS: u64 = 6;
const READERS: u64 = 4;

fn random_read(rng: &mut Rng) -> String {
    let x = rng.below(KEYS);
    match rng.below(7) {
        0 => format!("MATCH (n:A) WHERE n.k = {x} RETURN n.k, n.s, n.f"),
        1 => format!("MATCH (a:A)-[r:R]->(b:A) WHERE a.k = {x} RETURN b.k, r.w"),
        2 => "MATCH (n:A) RETURN count(n)".to_string(),
        3 => format!("MATCH (a:A)-[:R]->()-[:R]->(c) WHERE a.k = {x} RETURN c.k"),
        4 => "MATCH (n:B) RETURN n.k ORDER BY n.k".to_string(),
        5 => format!("MATCH p = (a:A)-[:R*1..2]->(b) WHERE a.k = {x} RETURN length(p)"),
        _ => "MATCH (n:A) WITH n.k AS k, count(*) AS c RETURN k, c ORDER BY k".to_string(),
    }
}

/// Reader client: every read must succeed. Returns the failures it saw.
async fn run_reader(addr: std::net::SocketAddr, seed: u64, ops: usize) -> Vec<String> {
    let mut rng = Rng(seed);
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut failures = Vec::new();
    for _ in 0..ops {
        let q = random_read(&mut rng);
        let resp = request(&mut s, &json!({"type": "query", "query": q})).await;
        if resp["type"] != "result" {
            failures.push(format!("{q} -> {resp}"));
        }
    }
    failures
}

/// Structural invariants of the graph itself.
fn graph_invariants(g: &ConcurrentGraph) -> Vec<String> {
    let mut problems = Vec::new();
    let nodes = g.all_nodes();
    let edges = g.all_edges();
    let node_ids: HashSet<u64> = nodes.iter().map(|n| n.id).collect();

    for e in &edges {
        if !node_ids.contains(&e.from) || !node_ids.contains(&e.to) {
            problems.push(format!("dangling edge {} ({} -> {})", e.id, e.from, e.to));
        }
    }

    // Adjacency sets must list exactly the edges touching each node.
    let mut expected_out: HashMap<u64, HashSet<u64>> = HashMap::new();
    let mut expected_in: HashMap<u64, HashSet<u64>> = HashMap::new();
    for e in &edges {
        expected_out.entry(e.from).or_default().insert(e.id);
        expected_in.entry(e.to).or_default().insert(e.id);
    }
    for id in &node_ids {
        let out: HashSet<u64> = g.outgoing_edges(*id).iter().map(|e| e.id).collect();
        let inc: HashSet<u64> = g.incoming_edges(*id).iter().map(|e| e.id).collect();
        if out != expected_out.remove(id).unwrap_or_default() {
            problems.push(format!(
                "outgoing adjacency of node {id} disagrees with edges"
            ));
        }
        if inc != expected_in.remove(id).unwrap_or_default() {
            problems.push(format!(
                "incoming adjacency of node {id} disagrees with edges"
            ));
        }
    }

    // Label index vs full scan.
    for label in ["A", "B", "U"] {
        let mut via_index = g.nodes_by_label(label);
        via_index.sort_unstable();
        via_index.dedup();
        let mut scan: Vec<u64> = nodes
            .iter()
            .filter(|n| n.has_label(label))
            .map(|n| n.id)
            .collect();
        scan.sort_unstable();
        if via_index != scan {
            problems.push(format!(
                "label index for :{label} has {} nodes, scan has {}",
                via_index.len(),
                scan.len()
            ));
        }
    }

    // UNIQUE(:U.id)
    let mut seen = HashSet::new();
    for n in nodes.iter().filter(|n| n.has_label("U")) {
        if let Some(PropertyValue::Int(id)) = n.properties.get("id")
            && !seen.insert(*id)
        {
            problems.push(format!("UNIQUE(:U.id) violated for id {id}"));
        }
    }
    problems
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_mixed_workload_keeps_graph_and_indexes_consistent() {
    let seed: u64 = std::env::var("MAHARIT_STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let ops: usize = std::env::var("MAHARIT_STRESS_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        max_connections: 64,
        read_timeout: Duration::from_secs(30),
        write_timeout: Duration::from_secs(30),
        ..Default::default()
    });
    let graph = server.graph_arc();
    tokio::spawn(async move {
        let _ = server.start_with_listener(listener).await;
    });

    let mut setup = TcpStream::connect(addr).await.unwrap();
    for q in [
        "CREATE INDEX ON :A(k)",
        "CREATE CONSTRAINT uq FOR (u:U) REQUIRE u.id IS UNIQUE",
    ] {
        request(&mut setup, &json!({"type": "query", "query": q})).await;
    }

    let writers: Vec<_> = (0..WRITERS)
        .map(|w| {
            tokio::spawn(run_client(
                addr,
                seed.wrapping_mul(7919).wrapping_add(w),
                ops,
            ))
        })
        .collect();
    let readers: Vec<_> = (0..READERS)
        .map(|r| {
            tokio::spawn(run_reader(
                addr,
                seed.wrapping_mul(104_729).wrapping_add(r),
                ops,
            ))
        })
        .collect();

    let mut logs = Vec::new();
    for w in writers {
        logs.push(w.await.unwrap());
    }
    let mut read_failures = Vec::new();
    for r in readers {
        read_failures.extend(r.await.unwrap());
    }

    let mut problems = Vec::new();
    if !read_failures.is_empty() {
        problems.push(format!(
            "{} reads failed during concurrent writes, e.g.:\n  {}",
            read_failures.len(),
            read_failures
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        ));
    }
    problems.extend(graph_invariants(&graph));
    let idx = index_mismatches(addr, &graph).await;
    if !idx.is_empty() {
        problems.push(format!("property index disagrees with graph: {idx:?}"));
    }
    let ping = request(&mut setup, &json!({"type": "ping"})).await;
    if ping["type"] != "pong" {
        problems.push(format!("server stopped answering: {ping}"));
    }

    let ok = logs
        .iter()
        .flatten()
        .filter(|l| l.ends_with("\"result\""))
        .count();
    let total: usize = logs.iter().map(Vec::len).sum();
    eprintln!(
        "stress seed {seed}: {total} writer ops ({ok} ok), {} reads, final {} nodes / {} edges",
        READERS as usize * ops,
        GraphBackend::node_count(graph.as_ref()),
        GraphBackend::edge_count(graph.as_ref())
    );
    assert!(
        problems.is_empty(),
        "stress seed {seed} (reproduce: MAHARIT_STRESS_SEED={seed} MAHARIT_STRESS_OPS={ops}):\n{}",
        problems.join("\n")
    );
}
