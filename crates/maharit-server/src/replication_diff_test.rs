//! Randomized differential test: leader vs follower (task121).
//!
//! Starts a real leader (`TcpServer` + `LeaderReplicationManager`) and a real
//! follower, drives the leader with several concurrent clients issuing random
//! writes (including transactions that commit or roll back and statements that
//! fail midway), then checks two invariants:
//!
//! 1. the follower's graph is identical to the leader's (IDs, labels,
//!    properties, edges), and
//! 2. the leader's property index agrees with a full scan of its graph.
//!
//! Every run is reproducible from its seed. On failure the seed and each
//! client's operation log are printed. Tune with environment variables:
//!
//! * `MAHARIT_DIFF_SEEDS` — comma-separated seeds (default `1,2,3,4,5`)
//! * `MAHARIT_DIFF_OPS` — operations per client (default `200`)
//! * `MAHARIT_DIFF_CLIENTS` — concurrent clients (default `4`)
//!
//! The defaults reproduce every replication bug fixed so far (bug/114–120 and
//! the WAL queue overflow; seed 3 needs 200 ops × 4 clients for the latter).
//! For a deeper soak: `MAHARIT_DIFF_SEEDS=$(seq -s, 1 100) cargo test -p maharit-server leader_and_follower`.

use std::sync::Arc;
use std::time::Duration;

use maharit_core::{ConcurrentGraph, GraphBackend, PropertyValue};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::replication::{
    FollowerReplicationManager, LeaderReplicationManager, NodeRole, ReplicationConfig,
};
use crate::tcp_server::{ServerConfig, TcpServer};

/// Range of the `k` property: small so random operations collide often.
const KEYS: u64 = 12;

/// Minimal deterministic PRNG (SplitMix64) — no extra dependency needed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Canonical, order-independent description of a graph's full contents.
pub(crate) type Fingerprint = (
    Vec<(u64, Vec<String>, Vec<String>)>,
    Vec<(u64, u64, u64, String, Vec<String>)>,
);

pub(crate) fn fingerprint(g: &ConcurrentGraph) -> Fingerprint {
    let props = |p: &std::collections::HashMap<String, PropertyValue>| {
        let mut v: Vec<String> = p.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
        v.sort();
        v
    };
    let mut nodes: Vec<_> = g
        .all_nodes()
        .into_iter()
        .map(|n| {
            let mut labels = n.labels.clone();
            labels.sort();
            (n.id, labels, props(&n.properties))
        })
        .collect();
    nodes.sort();
    let mut edges: Vec<_> = g
        .all_edges()
        .into_iter()
        .map(|e| (e.id, e.from, e.to, e.label.clone(), props(&e.properties)))
        .collect();
    edges.sort();
    (nodes, edges)
}

async fn request(stream: &mut TcpStream, req: &Value) -> Value {
    let body = serde_json::to_vec(req).unwrap();
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&body).await.unwrap();
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.unwrap();
    let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut buf).await.unwrap();
    serde_json::from_slice(&buf).unwrap()
}

/// Pick a random write statement over a small key space.
fn random_statement(rng: &mut Rng) -> String {
    let x = rng.below(KEYS);
    let y = rng.below(KEYS);
    match rng.below(14) {
        0..=2 => format!("CREATE (:A {{k: {x}, s: 'v{y}'}})"),
        3 => format!("MATCH (n:A) WHERE n.k = {x} SET n.k = {y}"),
        4 => format!("MATCH (n:A) WHERE n.k = {x} SET n:B"),
        5 => format!("MATCH (n:B) WHERE n.k = {x} REMOVE n:B"),
        6 => format!("MATCH (n:A) WHERE n.k = {x} REMOVE n.s"),
        7 => format!("MATCH (n:A) WHERE n.k = {x} SET n.f = {y}.0"),
        8 | 9 => format!(
            "MATCH (a:A), (b:A) WHERE a.k = {x} AND b.k = {y} CREATE (a)-[:R {{w: {y}.5}}]->(b)"
        ),
        10 => format!("MATCH (a:A)-[r:R]->(b:A) WHERE a.k = {x} SET r.w = {y}"),
        11 => format!("MATCH (a:A)-[r:R]->(b:A) WHERE a.k = {x} DELETE r"),
        12 => format!("MATCH (n:A) WHERE n.k = {x} DETACH DELETE n"),
        // Violates the UNIQUE constraint on :U(id) partway through when a
        // value repeats: the statement fails after partial changes.
        _ => format!("UNWIND [{x}, {y}, {x}] AS i CREATE (:U {{id: i}})"),
    }
}

/// One client's random workload. Returns its operation log.
async fn run_client(addr: std::net::SocketAddr, seed: u64, ops: usize) -> Vec<String> {
    let mut rng = Rng(seed);
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut log = Vec::with_capacity(ops);
    let mut tx: Option<u64> = None;

    for _ in 0..ops {
        match tx {
            None if rng.chance(12) => {
                let resp = request(&mut s, &json!({"type": "begin"})).await;
                tx = resp["txId"].as_u64();
                log.push(format!("BEGIN -> {:?}", tx));
                continue;
            }
            Some(id) if rng.chance(15) => {
                let kind = if rng.chance(50) { "commit" } else { "rollback" };
                let resp = request(&mut s, &json!({"type": kind, "txId": id})).await;
                log.push(format!(
                    "{} tx{id} -> {}",
                    kind.to_uppercase(),
                    resp["type"]
                ));
                tx = None;
                continue;
            }
            _ => {}
        }
        let stmt = random_statement(&mut rng);
        let mut req = json!({"type": "query", "query": stmt});
        if let Some(id) = tx {
            req["txId"] = json!(id);
        }
        let resp = request(&mut s, &req).await;
        log.push(format!(
            "{}{} -> {}",
            tx.map(|id| format!("[tx{id}] ")).unwrap_or_default(),
            stmt,
            resp["type"]
        ));
    }
    if let Some(id) = tx {
        let kind = if rng.chance(50) { "commit" } else { "rollback" };
        request(&mut s, &json!({"type": kind, "txId": id})).await;
        log.push(format!("{} tx{id} (final)", kind.to_uppercase()));
    }
    log
}

/// Property-index answers on the leader must match a full scan of its graph.
async fn index_mismatches(addr: std::net::SocketAddr, graph: &ConcurrentGraph) -> Vec<String> {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut out = Vec::new();
    for k in 0..KEYS {
        let resp = request(
            &mut s,
            &json!({"type": "query", "query": format!("MATCH (n:A) WHERE n.k = {k} RETURN n.k")}),
        )
        .await;
        let via_index = resp["rows"].as_array().map_or(usize::MAX, |r| r.len());
        let scan = graph
            .all_nodes()
            .iter()
            .filter(|n| {
                n.has_label("A") && n.properties.get("k") == Some(&PropertyValue::Int(k as i64))
            })
            .count();
        if via_index != scan {
            out.push(format!("k={k}: index={via_index} scan={scan}"));
        }
    }
    out
}

async fn run_seed(seed: u64, clients: u64, ops: usize) -> Result<(), String> {
    // Leader: replication channel + query server.
    let repl_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let repl_addr = repl_listener.local_addr().unwrap();
    let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
    repl.start_with_listener(repl_listener).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        read_timeout: Duration::from_secs(10),
        write_timeout: Duration::from_secs(10),
        ..Default::default()
    })
    .with_replication(Arc::clone(&repl));
    let leader_graph = server.graph_arc();
    tokio::spawn(async move {
        let _ = server.start_with_listener(listener).await;
    });

    // Follower.
    let follower_graph = Arc::new(ConcurrentGraph::new());
    let follower = FollowerReplicationManager::with_concurrent_graph(
        ReplicationConfig {
            role: NodeRole::Follower,
            node_id: format!("f-diff-{seed}"),
            replication_bind_address: "127.0.0.1:0".to_string(),
            leader_address: Some(repl_addr.to_string()),
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        },
        Arc::clone(&follower_graph),
    );
    follower.start().await.unwrap();
    for _ in 0..100 {
        if repl.get_follower_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Schema (replicated schema is out of scope; only the leader needs it).
    let mut setup = TcpStream::connect(addr).await.unwrap();
    for q in [
        "CREATE INDEX ON :A(k)",
        "CREATE CONSTRAINT uq FOR (u:U) REQUIRE u.id IS UNIQUE",
    ] {
        request(&mut setup, &json!({"type": "query", "query": q})).await;
    }

    // Concurrent random workload.
    let handles: Vec<_> = (0..clients)
        .map(|c| {
            tokio::spawn(run_client(
                addr,
                seed.wrapping_mul(1000).wrapping_add(c),
                ops,
            ))
        })
        .collect();
    let mut logs = Vec::new();
    for h in handles {
        logs.push(h.await.unwrap());
    }

    // Wait for the follower to converge.
    let expected = fingerprint(&leader_graph);
    let mut actual = fingerprint(&follower_graph);
    for _ in 0..250 {
        if actual == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        actual = fingerprint(&follower_graph);
    }

    let mut problems = Vec::new();
    if actual != expected {
        problems.push(describe_divergence(&expected, &actual));
    }
    let idx = index_mismatches(addr, &leader_graph).await;
    if !idx.is_empty() {
        problems.push(format!(
            "leader property index disagrees with graph: {idx:?}"
        ));
    }
    if repl.get_follower_count() != 1 {
        problems.push("follower was disconnected".to_string());
    }

    // Guard against a vacuous pass (e.g. every statement failing to parse).
    let all: Vec<&String> = logs.iter().flatten().collect();
    let ok = all.iter().filter(|l| l.ends_with("\"result\"")).count();
    let rollbacks = all.iter().filter(|l| l.starts_with("ROLLBACK")).count();
    eprintln!(
        "seed {seed}: {} ops, {ok} ok statements, {rollbacks} rollbacks, final {} nodes / {} edges",
        all.len(),
        expected.0.len(),
        expected.1.len()
    );
    if ok < all.len() / 3 {
        problems.push(format!(
            "workload too weak: {ok}/{} statements succeeded",
            all.len()
        ));
    }

    repl.shutdown();
    if problems.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "seed {seed} failed (reproduce: MAHARIT_DIFF_SEEDS={seed} MAHARIT_DIFF_OPS={ops} \
         MAHARIT_DIFF_CLIENTS={clients})\n{}\n",
        problems.join("\n")
    );
    for (c, log) in logs.iter().enumerate() {
        msg.push_str(&format!("--- client {c} ---\n{}\n", log.join("\n")));
    }
    Err(msg)
}

/// Up to five elements present in `a` but not in `b`.
fn only_in<T: PartialEq + std::fmt::Debug>(a: &[T], b: &[T]) -> Vec<String> {
    a.iter()
        .filter(|x| !b.contains(x))
        .take(5)
        .map(|x| format!("{x:?}"))
        .collect()
}

fn describe_divergence(expected: &Fingerprint, actual: &Fingerprint) -> String {
    [
        format!(
            "follower diverged: leader {} nodes / {} edges, follower {} nodes / {} edges",
            expected.0.len(),
            expected.1.len(),
            actual.0.len(),
            actual.1.len()
        ),
        format!(
            "  nodes only on leader:   {:?}",
            only_in(&expected.0, &actual.0)
        ),
        format!(
            "  nodes only on follower: {:?}",
            only_in(&actual.0, &expected.0)
        ),
        format!(
            "  edges only on leader:   {:?}",
            only_in(&expected.1, &actual.1)
        ),
        format!(
            "  edges only on follower: {:?}",
            only_in(&actual.1, &expected.1)
        ),
    ]
    .join("\n")
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leader_and_follower_converge_under_random_writes() {
    let seeds: Vec<u64> = std::env::var("MAHARIT_DIFF_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 3, 4, 5]);
    let ops = env_or("MAHARIT_DIFF_OPS", 200usize);
    let clients = env_or("MAHARIT_DIFF_CLIENTS", 4u64);

    let mut failures = Vec::new();
    for seed in seeds {
        if let Err(e) = run_seed(seed, clients, ops).await {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
