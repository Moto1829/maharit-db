//! クエリの実行（トランザクション内・ストリーミングを含む）、書き込みの WAL への記録と
//! レプリケーションへの送出、フォロワーの昇格。

use super::*;

/// Execute a streaming query and send results in chunks
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_streaming_query<S: AsyncWrite + Unpin>(
    socket: &mut S,
    graph: &Arc<ConcurrentGraph>,
    stats: &Arc<ServerStats>,
    query: &str,
    params: HashMap<String, Value>,
    chunk_size: usize,
    write_timeout: Duration,
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
) -> std::io::Result<()> {
    // Parse the query (reusing a cached AST when available). Bind the result to
    // a local so the mutex guard is released before any `.await` below.
    let parsed = ast_cache.lock().unwrap().get_or_parse(query);
    let stmt = match parsed {
        Ok(stmt) => stmt,
        Err(e) => {
            let response = Response::Error {
                message: format!("Parse error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    let is_write = !is_read_only(&stmt);
    let needs = managers.read_needs(query);

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = if is_write {
        Some(managers.write_gate.lock().await)
    } else {
        None
    };

    let record = sinks.record();
    let (exec_result, wal) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_with_params(graph, stmt, params, is_write, record, policy, Some(needs))
    })
    .await;
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return send_response(socket, &Response::Error { message }, write_timeout).await;
    }

    let result = match exec_result {
        Ok(r) => r,
        Err(e) => {
            let response = Response::Error {
                message: format!("Execution error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    let total_rows = result.rows.len();
    let columns = result.columns;
    let mut rows = result.rows.into_iter();
    let stream_id = stats.next_stream_id();

    // Send StreamStart
    let start_response = Response::StreamStart {
        stream_id,
        total_rows: Some(total_rows),
    };
    send_response(socket, &start_response, write_timeout).await?;

    // Send chunks
    let chunk_size = if chunk_size == 0 {
        DEFAULT_CHUNK_SIZE
    } else {
        chunk_size
    };

    // Convert each chunk only when it is sent (no full copy of all rows).
    let mut chunk_index = 0;
    loop {
        let chunk: Vec<HashMap<String, serde_json::Value>> = rows
            .by_ref()
            .take(chunk_size)
            .map(|row| {
                columns
                    .iter()
                    .zip(row.columns.iter())
                    .map(|(col, val)| (col.clone(), val.to_json()))
                    .collect()
            })
            .collect();
        if chunk.is_empty() {
            break;
        }
        let chunk_response = Response::StreamChunk {
            stream_id,
            chunk_index,
            rows: chunk,
        };
        send_response(socket, &chunk_response, write_timeout).await?;
        chunk_index += 1;
    }

    // Send StreamEnd
    let end_response = Response::StreamEnd {
        stream_id,
        total_rows,
    };
    send_response(socket, &end_response, write_timeout).await?;

    Ok(())
}

/// Convert the JSON `params` of a request to executor values.
pub(super) fn params_from_json(
    params: &HashMap<String, serde_json::Value>,
) -> HashMap<String, Value> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), Value::from_json(v)))
        .collect()
}

/// Run a statement on the blocking thread pool and wait for it.
///
/// Statements run synchronously and may take seconds. Running them on the async
/// worker threads would stall every other connection (queries, pings, new
/// connections, replication heartbeats) once as many heavy statements as there
/// are workers are in flight, so they get their own threads instead.
///
/// A writer keeps holding `write_gate` while it waits here, so writes are still
/// applied one at a time and in the order their WAL entries are published.
pub(super) async fn run_statement<R, F>(
    graph: &Arc<ConcurrentGraph>,
    managers: &Arc<SharedManagers>,
    f: F,
) -> R
where
    R: Send + 'static,
    F: FnOnce(&ConcurrentGraph, &SharedManagers) -> R + Send + 'static,
{
    let graph = Arc::clone(graph);
    let managers = Arc::clone(managers);
    match tokio::task::spawn_blocking(move || f(&graph, &managers)).await {
        Ok(result) => result,
        // The statement panicked: keep the panic instead of inventing a result.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// 実行結果を応答にする。
pub(super) fn result_response(result: maharit_query::ResultSet, format: RowFormat) -> Response {
    match format {
        RowFormat::Map => {
            let columns = result.columns;
            let rows = result
                .rows
                .into_iter()
                .map(|row| {
                    columns
                        .iter()
                        .zip(row.columns.iter())
                        .map(|(col, val)| (col.clone(), val.to_json()))
                        .collect()
                })
                .collect();
            Response::Result { rows }
        }
        RowFormat::Array => Response::Table {
            rows: result
                .rows
                .iter()
                .map(|row| row.columns.iter().map(Value::to_json).collect())
                .collect(),
            columns: result.columns,
        },
    }
}

// ── Transaction-aware query execution ────────────────────────────────────────

/// Execute a write query within a transaction: execute → record undo log → replicate.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_query_with_tx(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    params: HashMap<String, Value>,
    tx_id: u64,
    tx_manager: &TransactionManager,
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
    format: RowFormat,
) -> Response {
    let stmt = match ast_cache.lock().unwrap().get_or_parse(query) {
        Ok(s) => s,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            };
        }
    };

    let is_write = !is_read_only(&stmt);

    if !is_write {
        return execute_query(
            graph, query, params, managers, sinks, ast_cache, policy, format,
        )
        .await;
    }

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = managers.write_gate.lock().await;

    // Refuse to write under a transaction that is unknown or already
    // finished: its changes could never be rolled back.
    if !tx_manager.is_active(tx_id) {
        return Response::Error {
            message: format!("Execution error: transaction {} is not active", tx_id),
        };
    }
    if tx_manager.is_read_only(tx_id) {
        return Response::Error {
            message: format!(
                "Execution error: transaction {} is read-only: cannot run a write query",
                tx_id
            ),
        };
    }

    let record = sinks.record();
    let (exec_result, wal, undo) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_in_tx(graph, stmt, params, record, policy)
    })
    .await;
    // A failed statement has already been undone (no undo records left).
    if let Err(e) = tx_manager.record_undo(tx_id, undo) {
        tracing::error!(tx_id, error = %e, "failed to record undo log");
    }
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return Response::Error { message };
    }

    match exec_result {
        Ok(result) => result_response(result, format),
        Err(e) => Response::Error {
            message: format!("Execution error: {}", e),
        },
    }
}

/// Execute a query and return the response.
///
/// ConcurrentGraph has interior mutability via DashMap, so both read and write
/// queries use `Executor::new_concurrent` without any async locking.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_query(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    params: HashMap<String, Value>,
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
    format: RowFormat,
) -> Response {
    // Reuse a previously parsed AST for identical query text.
    let stmt = match ast_cache.lock().unwrap().get_or_parse(query) {
        Ok(stmt) => stmt,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            };
        }
    };

    let is_write = !is_read_only(&stmt);
    let needs = managers.read_needs(query);

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = if is_write {
        Some(managers.write_gate.lock().await)
    } else {
        None
    };

    let record = sinks.record();
    let (exec_result, wal) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_with_params(graph, stmt, params, is_write, record, policy, Some(needs))
    })
    .await;
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return Response::Error { message };
    }

    match exec_result {
        Ok(result) => result_response(result, format),
        Err(e) => Response::Error {
            message: format!("Execution error: {}", e),
        },
    }
}

/// Snapshot source for a leader: this server's graph, write gate and schema.
pub(super) fn snapshot_source(
    graph: &Arc<ConcurrentGraph>,
    managers: &Arc<SharedManagers>,
) -> SnapshotSource {
    let schema_managers = Arc::clone(managers);
    SnapshotSource {
        graph: Arc::clone(graph),
        write_gate: Arc::clone(&managers.write_gate),
        schema: Arc::new(move || schema_managers.export_schema()),
    }
}

/// Error for a write sent to a follower that has not been promoted, or `None`
/// when the query may run here.
pub(super) fn reject_write_on_follower(
    follower: Option<&FollowerReplicationManager>,
    ast_cache: &Arc<Mutex<AstCache>>,
    query: &str,
) -> Option<Response> {
    let follower = follower?;
    if follower.is_promoted() {
        return None;
    }
    let stmt = ast_cache.lock().unwrap().get_or_parse(query).ok()?;
    if is_read_only(&stmt) {
        return None;
    }
    Some(Response::Error {
        message: "this node is a read-only follower: send writes to the leader".to_string(),
    })
}

/// Turn this follower into a leader: stop following, make the data file and
/// WAL current, start serving replication (if configured) and accept writes.
pub(super) async fn promote_to_leader(
    follower: Option<&FollowerReplicationManager>,
    promotion: Option<&Promotion>,
    replication: &SharedLeader,
    graph: &Arc<ConcurrentGraph>,
    managers: &Arc<SharedManagers>,
    durability: Option<&Arc<Durability>>,
) -> Response {
    let Some(follower) = follower else {
        return Response::Error {
            message: "this node is not a follower".to_string(),
        };
    };
    if follower.is_promoted() {
        return Response::Error {
            message: "this node has already been promoted".to_string(),
        };
    }
    let Some(promotion) = promotion else {
        return Response::Error {
            message: "promotion is not configured on this node".to_string(),
        };
    };
    // Bind first: if the address is taken, nothing has changed yet.
    let listener = match &promotion.replication {
        Some(cfg) => match TcpListener::bind(&cfg.replication_bind_address).await {
            Ok(l) => Some(l),
            Err(e) => {
                return Response::Error {
                    message: format!(
                        "cannot listen for followers on {}: {e}",
                        cfg.replication_bind_address
                    ),
                };
            }
        },
        None => None,
    };

    let _gate = managers.write_gate.lock().await;
    follower.stop().await;

    // Everything replicated so far is only in memory: save it as the snapshot
    // and start an empty WAL for it before the first write.
    if let Some(d) = durability {
        let checkpointer = Checkpointer {
            graph: Arc::clone(graph),
            managers: Arc::clone(managers),
            durability: Arc::clone(d),
        };
        if let Err(e) = checkpointer.checkpoint_gated().await {
            tracing::error!(error = %e, "checkpoint during promotion failed");
            return Response::Error {
                message: format!(
                    "promotion stopped replication but failed to save the data file: {e}"
                ),
            };
        }
    }

    let mut replication_bind = None;
    if let (Some(cfg), Some(listener)) = (&promotion.replication, listener) {
        let mut leader = LeaderReplicationManager::new(cfg.clone());
        if let Some(tls) = &promotion.tls {
            leader = leader.with_tls(tls.clone());
        }
        leader.set_snapshot_source(snapshot_source(graph, managers));
        let leader = Arc::new(leader);
        replication_bind = Some(cfg.replication_bind_address.clone());
        if let Err(e) = leader.start_with_listener(listener).await {
            tracing::error!(error = %e, "failed to start the replication listener");
        }
        *replication.write().unwrap_or_else(|e| e.into_inner()) = Some(leader);
    }
    follower.mark_promoted();
    tracing::warn!(replication_bind = ?replication_bind, "promoted to leader");
    println!(
        "Promoted to leader{}",
        replication_bind
            .as_deref()
            .map(|b| format!(" (replication on {b})"))
            .unwrap_or_default()
    );
    Response::Promoted { replication_bind }
}

/// Forward the mutations recorded during a write statement to the WAL, in
/// execution order.
pub(super) async fn emit_wal_entries(
    entries: Vec<WalEntryData>,
    replication: &LeaderReplicationManager,
) {
    for entry in entries {
        replication.append_wal_entry(entry).await;
    }
}
