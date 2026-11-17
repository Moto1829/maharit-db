//! 1 接続の処理: メッセージの読み書きと、リクエストの種類ごとの振り分け、
//! 切断時のトランザクションの取り消し。

use super::*;

/// Server state shared with one connection task.
pub(super) struct ConnectionContext {
    pub(super) graph: Arc<ConcurrentGraph>,
    pub(super) stats: Arc<ServerStats>,
    pub(super) shutdown: Arc<AtomicBool>,
    pub(super) tx_manager: Arc<TransactionManager>,
    pub(super) managers: Arc<SharedManagers>,
    pub(super) config: ServerConfig,
    pub(super) replication: SharedLeader,
    pub(super) follower: Option<Arc<FollowerReplicationManager>>,
    pub(super) promotion: Option<Arc<Promotion>>,
    pub(super) auth: Arc<Mutex<crate::auth::AuthManager>>,
    pub(super) ast_cache: Arc<Mutex<AstCache>>,
    pub(super) durability: Option<Arc<Durability>>,
    pub(super) metrics: Arc<crate::metrics::Metrics>,
    pub(super) audit: Option<Arc<crate::audit::AuditLogger>>,
}

/// Handle a single client connection (plain TCP or TLS).
pub(super) async fn handle_connection<S>(
    mut socket: S,
    ctx: ConnectionContext,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ConnectionContext {
        graph,
        stats,
        shutdown,
        tx_manager,
        managers,
        config,
        replication,
        follower,
        promotion,
        auth,
        ast_cache,
        durability,
        metrics,
        audit,
    } = ctx;
    let audit = audit.as_deref();
    let mut buffer = BytesMut::with_capacity(4096);

    // Transactions begun on this connection and not finished yet: rolled back
    // when the connection ends (bug/159).
    let mut open_txs: Vec<u64> = Vec::new();
    let result: std::io::Result<()> = async {
        loop {
            // Read per request: a promoted follower starts publishing to its own
            // followers from the next request on.
            let leader = replication
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let sinks = WriteSinks {
                replication: leader.as_deref(),
                wal: durability.as_ref().map(|d| &d.wal),
            };

            if shutdown.load(Ordering::SeqCst) {
                send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                break;
            }

            // Read with timeout
            let read_result = tokio::select! {
                result = read_message(&mut socket, &mut buffer, config.read_timeout) => result,
                _ = shutdown_rx.recv() => {
                    send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                    break;
                }
            };

            let message = match read_result {
                Ok(Some(msg)) => msg,
                Ok(None) => break, // Connection closed
                Err(e) => {
                    let response = Response::Error {
                        message: e.to_string(),
                    };
                    send_response(&mut socket, &response, config.write_timeout).await?;
                    continue;
                }
            };

            // Parse request
            let request: Request = match serde_json::from_slice(&message) {
                Ok(req) => req,
                Err(e) => {
                    let response = Response::Error {
                        message: format!("Invalid request: {}", e),
                    };
                    send_response(&mut socket, &response, config.write_timeout).await?;
                    continue;
                }
            };

            // Handle request
            // Metrics: classify the query before `request` is consumed.
            let measured = match &request {
                Request::Query { query, .. } | Request::StreamQuery { query, .. } => {
                    Some(crate::metrics::QueryType::of_query(query))
                }
                _ => None,
            };
            let started = std::time::Instant::now();

            let response = match request {
                Request::Login { username, password } => {
                    let mut mgr = auth.lock().unwrap();
                    match mgr.authenticate(&username, &password) {
                        Ok(token) => {
                            audit_log(
                                audit,
                                crate::audit::AuthEvent::LoginSuccess {
                                    username: username.clone(),
                                },
                            );
                            let role = mgr
                                .validate_session(&token)
                                .map(|s| role_label(&s.role))
                                .unwrap_or_else(|_| "unknown".to_string());
                            let expires_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                                + DEFAULT_SESSION_TIMEOUT_SECS;
                            Response::LoggedIn {
                                session_token: token,
                                role,
                                expires_at,
                            }
                        }
                        Err(e) => {
                            audit_log(
                                audit,
                                crate::audit::AuthEvent::LoginFailure {
                                    username: username.clone(),
                                },
                            );
                            Response::AuthError {
                                message: format!("{}", e),
                            }
                        }
                    }
                }
                Request::Query {
                    query,
                    params,
                    tx_id,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) => {
                        if let Some(resp) = try_handle_user_management(
                            &query,
                            &auth,
                            &ast_cache,
                            role,
                            config.require_auth,
                            audit,
                            &user,
                        ) {
                            resp
                        } else if let Some(resp) = authorize_query(role, &query) {
                            audit_denied(audit, &user, "write");
                            resp
                        } else if let Some(resp) =
                            reject_write_on_follower(follower.as_deref(), &ast_cache, &query)
                        {
                            resp
                        } else {
                            stats.total_queries.fetch_add(1, Ordering::SeqCst);
                            // `.instrument` (not `span.enter()`): the guard of
                            // `enter` must not be held across `.await`, or the
                            // span can be closed on another worker thread.
                            let span = tracing::info_span!("query", query = %query);
                            let params = params_from_json(&params);
                            let policy = session_policy(&auth, config.require_auth, role, &user);
                            async {
                                let start = std::time::Instant::now();
                                let resp = match tx_id {
                                    Some(id) => {
                                        execute_query_with_tx(
                                            &graph,
                                            &query,
                                            params,
                                            id,
                                            &tx_manager,
                                            &managers,
                                            sinks,
                                            &ast_cache,
                                            policy,
                                        )
                                        .await
                                    }
                                    None => {
                                        execute_query(
                                            &graph, &query, params, &managers, sinks, &ast_cache,
                                            policy,
                                        )
                                        .await
                                    }
                                };
                                if is_access_denied(&resp) {
                                    audit_denied(audit, &user, "acl_write");
                                }
                                tracing::info!(
                                    duration_us = start.elapsed().as_micros() as u64,
                                    "query completed"
                                );
                                resp
                            }
                            .instrument(span)
                            .await
                        }
                    }
                },
                Request::StreamQuery {
                    query,
                    params,
                    tx_id: _,
                    chunk_size,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) if authorize_query(role, &query).is_some() => {
                        audit_denied(audit, &user, "write");
                        authorize_query(role, &query).unwrap()
                    }
                    Ok(_)
                        if reject_write_on_follower(follower.as_deref(), &ast_cache, &query)
                            .is_some() =>
                    {
                        reject_write_on_follower(follower.as_deref(), &ast_cache, &query).unwrap()
                    }
                    Ok((role, user)) => {
                        stats.total_queries.fetch_add(1, Ordering::SeqCst);
                        tracing::info!(query = %query, "streaming query");
                        let policy = session_policy(&auth, config.require_auth, role, &user);
                        // Execute streaming query
                        if let Err(e) = execute_streaming_query(
                            &mut socket,
                            &graph,
                            &stats,
                            &query,
                            params_from_json(&params),
                            chunk_size,
                            config.write_timeout,
                            &managers,
                            sinks,
                            &ast_cache,
                            policy,
                        )
                        .await
                        {
                            Response::Error {
                                message: format!("Streaming error: {}", e),
                            }
                        } else {
                            // Streaming responses already sent, continue to next request
                            continue;
                        }
                    }
                },
                Request::Ping => Response::Pong,
                Request::Stats => {
                    // Report replication status (role, node id, current LSN, follower
                    // count, leader liveness) when this node participates in
                    // replication as a follower or leader.
                    let replication_status = match (leader.as_ref(), follower.as_ref()) {
                        (Some(l), _) => Some(ReplicationStatus::from(l.get_stats())),
                        (None, Some(f)) if f.is_promoted() => Some(ReplicationStatus {
                            role: "leader".to_string(),
                            node_id: f.get_stats().node_id,
                            current_lsn: 0,
                            follower_count: 0,
                            is_leader_alive: true,
                        }),
                        (None, Some(f)) => Some(ReplicationStatus::from(f.get_stats())),
                        (None, None) => None,
                    };
                    Response::Stats {
                        connections: stats.current_connections.load(Ordering::SeqCst),
                        total_queries: stats.total_queries.load(Ordering::SeqCst),
                        nodes: graph.node_count(),
                        edges: graph.edge_count(),
                        replication: replication_status,
                    }
                }
                Request::Promote { session_token } => {
                    match check_session_user(config.require_auth, &auth, &session_token) {
                        Err(resp) => resp,
                        Ok((role, user)) if role != crate::auth::Role::Admin => {
                            audit_denied(audit, &user, "promote");
                            Response::AuthError {
                                message: "promote requires the admin role".to_string(),
                            }
                        }
                        Ok(_) => {
                            promote_to_leader(
                                follower.as_deref(),
                                promotion.as_deref(),
                                &replication,
                                &graph,
                                &managers,
                                durability.as_ref(),
                            )
                            .await
                        }
                    }
                }
                Request::Disconnect => {
                    send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                    break;
                }
                Request::BeginTransaction {
                    read_only,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) => {
                        // 書き込みトランザクションの開始には Write 権限を要求する。
                        if !read_only
                            && crate::auth::AuthManager::check_role_permission(
                                role,
                                crate::auth::Operation::Write,
                            )
                            .is_err()
                        {
                            audit_denied(audit, &user, "begin_write_transaction");
                            Response::AuthError {
                                message: "permission denied: role cannot begin a write transaction"
                                    .to_string(),
                            }
                        } else {
                            let tx_id = if read_only {
                                tx_manager.begin_read_only()
                            } else {
                                tx_manager.begin()
                            };
                            Response::TransactionBegun { tx_id }
                        }
                    }
                },
                Request::Commit {
                    tx_id,
                    session_token,
                } => match check_session(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok(_) => match tx_manager.commit(tx_id) {
                        Ok(()) => Response::Committed { tx_id },
                        Err(e) => Response::Error {
                            message: format!("Commit failed: {}", e),
                        },
                    },
                },
                Request::Rollback {
                    tx_id,
                    session_token,
                } => match check_session(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok(_) => {
                        rollback_transaction(&graph, &managers, &tx_manager, sinks, tx_id).await
                    }
                },
            };

            if let Some(query_type) = measured {
                metrics.record_query(query_type, started.elapsed());
                if matches!(
                    response,
                    Response::Error { .. } | Response::AuthError { .. }
                ) {
                    metrics.record_error();
                }
                metrics.set_node_count(graph.node_count() as i64);
                metrics.set_edge_count(graph.edge_count() as i64);
            }

            match &response {
                Response::TransactionBegun { tx_id } => open_txs.push(*tx_id),
                Response::Committed { tx_id } | Response::RolledBack { tx_id } => {
                    open_txs.retain(|t| t != tx_id)
                }
                _ => {}
            }

            send_response(&mut socket, &response, config.write_timeout).await?;
        }
        Ok(())
    }
    .await;

    if !open_txs.is_empty() {
        let leader = replication
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let sinks = WriteSinks {
            replication: leader.as_deref(),
            wal: durability.as_ref().map(|d| &d.wal),
        };
        for tx_id in open_txs {
            if !tx_manager.is_active(tx_id) {
                continue;
            }
            if let Response::Error { message } =
                rollback_transaction(&graph, &managers, &tx_manager, sinks, tx_id).await
            {
                tracing::warn!(tx_id, error = %message, "failed to roll back on disconnect");
            } else {
                tracing::info!(tx_id, "rolled back unfinished transaction on disconnect");
            }
        }
    }

    result
}

/// Roll back `tx_id`: apply the compensating changes, write them to the WAL and
/// send them to followers.
pub(super) async fn rollback_transaction(
    graph: &Arc<ConcurrentGraph>,
    managers: &SharedManagers,
    tx_manager: &TransactionManager,
    sinks: WriteSinks<'_>,
    tx_id: u64,
) -> Response {
    // Serialize with writers so the compensating WAL entries are ordered after
    // the statements they undo.
    let write_guard = managers.write_gate.lock().await;
    match managers.rollback(graph, tx_manager, tx_id, sinks.record()) {
        Ok(wal) => {
            let ticket = sinks.publish(wal).await;
            drop(write_guard);
            let durable = match ticket {
                Ok(t) => sinks.wait_durable(t).await,
                Err(e) => Err(e),
            };
            match durable {
                Ok(()) => Response::RolledBack { tx_id },
                Err(message) => Response::Error { message },
            }
        }
        Err(e) => Response::Error {
            message: format!("Rollback failed: {}", e),
        },
    }
}

/// Read a length-prefixed message from the socket
pub(super) async fn read_message<S: AsyncRead + Unpin>(
    socket: &mut S,
    buffer: &mut BytesMut,
    read_timeout: Duration,
) -> std::io::Result<Option<Vec<u8>>> {
    loop {
        // Check if we have a complete message in the buffer
        if buffer.len() >= 4 {
            let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

            // 宣言された長さが上限を超える場合は、データを蓄積する前に切断する。
            // これがないと最大約 4 GiB のバッファ確保を強制されメモリ枯渇 DoS になる。
            if len > MAX_MESSAGE_SIZE {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "message length {} exceeds maximum {}",
                        len, MAX_MESSAGE_SIZE
                    ),
                ));
            }

            if buffer.len() >= 4 + len {
                buffer.advance(4);
                let message = buffer.split_to(len).to_vec();
                return Ok(Some(message));
            }
        }

        // Read more data
        let n = match timeout(read_timeout, socket.read_buf(buffer)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Read timeout",
                ));
            }
        };

        if n == 0 {
            return Ok(None); // Connection closed
        }
    }
}

/// Send a response with length prefix
pub(super) async fn send_response<S: AsyncWrite + Unpin>(
    socket: &mut S,
    response: &Response,
    write_timeout: Duration,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let len = json.len() as u32;
    let len_bytes = len.to_be_bytes();

    let write_future = async {
        socket.write_all(&len_bytes).await?;
        socket.write_all(&json).await?;
        socket.flush().await
    };

    match timeout(write_timeout, write_future).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Write timeout",
        )),
    }
}
