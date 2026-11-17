//! サーバーが接続をまたいで持つ状態: 永続化（WAL・スナップショット・チェックポイント）、
//! 書き込みの記録先、制約・索引の共有マネージャ、昇格の手順。

use super::*;

/// The leader replication manager, if this node currently is a leader. Set at
/// startup on a leader, or when a follower is promoted.
pub type SharedLeader = Arc<std::sync::RwLock<Option<Arc<LeaderReplicationManager>>>>;

/// What a follower does to become a leader.
pub struct Promotion {
    /// Leader replication settings (role, bind address, secret, node ID).
    /// `None` makes the promoted node a standalone leader without followers.
    pub replication: Option<ReplicationConfig>,
    /// TLS for the replication listener.
    pub tls: Option<ReloadableTlsAcceptor>,
}

/// On-disk durability for a server: the WAL and the snapshot it applies to.
pub struct Durability {
    pub(super) wal: Arc<DurableWal>,
    pub(super) data_path: std::path::PathBuf,
    /// Take a checkpoint (save the snapshot, empty the WAL) once the WAL grows
    /// past this many bytes. 0 disables size-triggered checkpoints.
    pub(super) checkpoint_bytes: u64,
}

impl Durability {
    pub fn new(
        wal: DurableWal,
        data_path: impl Into<std::path::PathBuf>,
        checkpoint_bytes: u64,
    ) -> Self {
        Self {
            wal: Arc::new(wal),
            data_path: data_path.into(),
            checkpoint_bytes,
        }
    }
}

/// Writes the graph and its schema to a data file.
#[derive(Clone)]
pub struct SnapshotWriter {
    pub(super) graph: Arc<ConcurrentGraph>,
    pub(super) managers: Arc<SharedManagers>,
}

impl SnapshotWriter {
    /// Save atomically to `path`, with writers paused.
    pub async fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let _gate = self.managers.write_gate.lock().await;
        save_snapshot(&self.graph, &self.managers, path)
    }
}

/// Save `graph` and the managers' schema to `path` (v5 data file).
pub(super) fn save_snapshot(
    graph: &ConcurrentGraph,
    managers: &SharedManagers,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let schema = managers.export_schema();
    let schema_json = if schema.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&schema).map_err(std::io::Error::other)?
    };
    maharit_storage::PersistentStorage::save_concurrent_with_schema(graph, &schema_json, path)
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Saves a snapshot and empties the WAL, with writers paused.
#[derive(Clone)]
pub struct Checkpointer {
    pub(super) graph: Arc<ConcurrentGraph>,
    pub(super) managers: Arc<SharedManagers>,
    pub(super) durability: Arc<Durability>,
}

impl Checkpointer {
    /// Write the snapshot atomically, then start an empty WAL for it. Holds the
    /// write gate throughout so no write lands between the two steps.
    pub async fn checkpoint(&self) -> std::io::Result<()> {
        let _gate = self.managers.write_gate.lock().await;
        self.checkpoint_gated().await
    }

    /// [`checkpoint`](Self::checkpoint) for a caller already holding the
    /// write gate.
    pub(super) async fn checkpoint_gated(&self) -> std::io::Result<()> {
        let graph = Arc::clone(&self.graph);
        let managers = Arc::clone(&self.managers);
        let durability = Arc::clone(&self.durability);
        tokio::task::spawn_blocking(move || {
            let path = &durability.data_path;
            save_snapshot(&graph, &managers, path)?;
            durability.wal.reset(snapshot_digest(path)?)
        })
        .await
        .map_err(std::io::Error::other)?
    }
}

/// Where a write statement's mutations go: the on-disk WAL and/or followers.
#[derive(Clone, Copy)]
pub(super) struct WriteSinks<'a> {
    pub(super) replication: Option<&'a LeaderReplicationManager>,
    pub(super) wal: Option<&'a Arc<DurableWal>>,
}

impl WriteSinks<'_> {
    /// Whether executors must record mutations at all.
    pub(super) fn record(&self) -> bool {
        self.replication.is_some() || self.wal.is_some()
    }

    /// Append the statement's mutations to the WAL and send them to followers.
    /// Call while holding the write gate. Returns the WAL offset to pass to
    /// [`wait_durable`](Self::wait_durable) once the gate is released.
    pub(super) async fn publish(&self, entries: Vec<WalEntryData>) -> Result<Option<u64>, String> {
        let mut ticket = Ok(None);
        if let Some(wal) = self.wal
            && !entries.is_empty()
        {
            ticket = wal.append(&entries).map(Some).map_err(|e| {
                tracing::error!(error = %e, "failed to append to WAL");
                format!("write was applied but could not be written to the WAL: {e}")
            });
        }
        if let Some(repl) = self.replication {
            emit_wal_entries(entries, repl).await;
        }
        ticket
    }

    /// Wait until the WAL is on disk up to `ticket` (group commit).
    pub(super) async fn wait_durable(&self, ticket: Option<u64>) -> Result<(), String> {
        let (Some(wal), Some(offset)) = (self.wal, ticket) else {
            return Ok(());
        };
        let wal = Arc::clone(wal);
        tokio::task::spawn_blocking(move || wal.sync_to(offset))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
            .map_err(|e| {
                tracing::error!(error = %e, "failed to sync WAL");
                format!("write was applied but the WAL could not be synced: {e}")
            })
    }
}

/// Schema/index state shared by every query on the server.
///
/// Read-only queries borrow the managers under read locks (no per-query clone).
/// Write queries are serialized by `write_gate` and update the managers in place
/// under write locks, so concurrent writers can no longer overwrite each other's
/// index updates (lost update) and UNIQUE checks see every committed write.
/// The gate is an async mutex so it can also cover the WAL emission that follows
/// execution, keeping replication order identical to execution order.
pub(super) struct SharedManagers {
    pub(super) constraints: RwLock<ConstraintManager>,
    pub(super) fulltext: RwLock<FulltextManager>,
    /// Shared property (B-tree) index so that `CREATE INDEX` and subsequent
    /// index-accelerated lookups survive between requests.
    pub(super) property_index: RwLock<PropertyIndex>,
    pub(super) write_gate: Arc<tokio::sync::Mutex<()>>,
}

impl SharedManagers {
    pub(super) fn new() -> Self {
        Self {
            constraints: RwLock::new(ConstraintManager::new()),
            fulltext: RwLock::new(FulltextManager::new()),
            property_index: RwLock::new(PropertyIndex::new()),
            write_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Run `stmt` against `graph` with these managers.
    ///
    /// Write statements must be executed while holding `write_gate`. For writes
    /// with `record_wal`, the returned log holds every graph mutation in
    /// execution order. A statement that fails midway is undone and returns an
    /// empty log, so the WAL and followers never see its partial changes.
    #[cfg(test)]
    pub(super) fn execute(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        is_write: bool,
        record_wal: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
    ) {
        self.execute_with_params(graph, stmt, HashMap::new(), is_write, record_wal, None)
    }

    /// [`execute`](Self::execute) with `$name` query parameters.
    pub(super) fn execute_with_params(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
        policy: Option<AccessPolicy>,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
    ) {
        let (result, wal, _) = self.run(graph, stmt, params, is_write, record_wal, false, policy);
        (result, wal)
    }

    /// Run a write statement inside a transaction: like [`execute`] but also
    /// returns the undo records needed to roll the statement back.
    ///
    /// Must be called while holding `write_gate`.
    pub(super) fn execute_in_tx(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        record_wal: bool,
        policy: Option<AccessPolicy>,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        self.run(graph, stmt, params, true, record_wal, true, policy)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
        capture_undo: bool,
        policy: Option<AccessPolicy>,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        // Lock order is always constraints → fulltext → property_index.
        // A poisoned lock only means another query panicked mid-execution; the
        // managers are still structurally valid, so keep serving.
        if is_write {
            use maharit_query::ast::Statement;
            let is_ddl = matches!(
                stmt,
                Statement::CreateConstraint(_)
                    | Statement::DropConstraint(_)
                    | Statement::CreateIndex(_)
                    | Statement::DropIndex(_)
                    | Statement::CreateFulltextIndex(_)
                    | Statement::DropFulltextIndex(_)
            );
            let mut cm = self.constraints.write().unwrap_or_else(|e| e.into_inner());
            let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
            let mut pi = self
                .property_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            // Undo records are always captured so that a statement that fails
            // midway can be undone: a statement either applies completely or
            // not at all (bug/160).
            let mut recorder = RecordingGraph::new(graph, record_wal).with_undo();
            // スキーマ変更（索引・制約）は ACL を通さずに全ノードを対象にする
            let policy = if is_ddl { None } else { policy };
            let result =
                Executor::new_with_backend_exclusive(&mut recorder, &mut cm, &mut fm, &mut pi)
                    .with_access_policy(policy)
                    .execute_with_params(stmt, params);
            let (mut wal, mut undo) = recorder.into_parts();
            if result.is_err() {
                if !undo.is_empty() {
                    // Not recorded: the partial changes never reach the WAL or
                    // followers, so nothing needs to be compensated there.
                    let mut reverter = RecordingGraph::new(graph, false);
                    let touched = maharit_storage::apply_undo(&undo, &mut reverter);
                    crate::schema::reindex_nodes(graph, touched, &mut fm, &mut pi);
                }
                wal.clear();
                undo.clear();
            } else if !capture_undo {
                undo.clear();
            }
            // Log the whole schema after a DDL statement so that WAL replay
            // and followers end up with the same constraints and indexes.
            if is_ddl && record_wal && result.is_ok() {
                wal.push(WalEntryData::SetSchema {
                    schema: Schema::export(&cm, &fm, &pi),
                });
            }
            (result, wal, undo)
        } else {
            let cm = self.constraints.read().unwrap_or_else(|e| e.into_inner());
            let fm = self.fulltext.read().unwrap_or_else(|e| e.into_inner());
            let pi = self
                .property_index
                .read()
                .unwrap_or_else(|e| e.into_inner());
            // SAFETY: ConcurrentGraph has interior mutability via DashMap; the
            // executor uses the raw pointer only during this synchronous call.
            let mut executor = unsafe { Executor::new_concurrent_shared(graph, &cm, &fm, &pi) }
                .with_access_policy(policy);
            (
                executor.execute_with_params(stmt, params),
                Vec::new(),
                Vec::new(),
            )
        }
    }

    /// Roll back transaction `tx_id`: apply its undo log to the graph, bring
    /// the property / fulltext indexes of every touched node back in line with
    /// the restored state, and return the compensating mutations as WAL
    /// entries so followers roll back too.
    ///
    /// Must be called while holding `write_gate`.
    pub(super) fn rollback(
        &self,
        graph: &ConcurrentGraph,
        tx_manager: &TransactionManager,
        tx_id: u64,
        record_wal: bool,
    ) -> Result<Vec<WalEntryData>, maharit_storage::TransactionError> {
        let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
        let mut pi = self
            .property_index
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let mut recorder = RecordingGraph::new(graph, record_wal);
        let touched = tx_manager.rollback_backend(tx_id, &mut recorder)?;
        crate::schema::reindex_nodes(graph, touched, &mut fm, &mut pi);
        Ok(recorder.into_log())
    }

    /// The current constraint and index definitions.
    pub(super) fn export_schema(&self) -> Schema {
        let cm = self.constraints.read().unwrap_or_else(|e| e.into_inner());
        let fm = self.fulltext.read().unwrap_or_else(|e| e.into_inner());
        let pi = self
            .property_index
            .read()
            .unwrap_or_else(|e| e.into_inner());
        Schema::export(&cm, &fm, &pi)
    }

    /// Replace the definitions with `schema` and rebuild the indexes.
    pub(super) fn install_schema(
        &self,
        graph: &ConcurrentGraph,
        schema: &Schema,
    ) -> Result<(), String> {
        let mut cm = self.constraints.write().unwrap_or_else(|e| e.into_inner());
        let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
        let mut pi = self
            .property_index
            .write()
            .unwrap_or_else(|e| e.into_inner());
        schema.install(graph, &mut cm, &mut fm, &mut pi)
    }
}

impl ReplicaApply for SharedManagers {
    fn applied(&self, graph: &ConcurrentGraph, entry: &WalEntryData) {
        if let WalEntryData::SetSchema { schema } = entry {
            if let Err(e) = self.install_schema(graph, schema) {
                tracing::error!(error = %e, "replication: failed to apply schema");
            }
            return;
        }
        if let Some(id) = crate::replication::touched_node(entry) {
            let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
            let mut pi = self
                .property_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if pi.has_any_index() || !fm.list_indexes().is_empty() {
                crate::schema::reindex_nodes(graph, [id], &mut fm, &mut pi);
            }
        }
    }

    fn snapshot_applied(&self, graph: &ConcurrentGraph, schema: &Schema) {
        if let Err(e) = self.install_schema(graph, schema) {
            tracing::error!(error = %e, "replication: failed to apply schema from snapshot");
        }
    }
}
