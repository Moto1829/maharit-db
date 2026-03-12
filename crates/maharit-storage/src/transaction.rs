//! Transaction support with ACID properties
//!
//! Provides transaction management with:
//! - BEGIN / COMMIT / ROLLBACK operations
//! - Transaction ID management
//! - Pessimistic locking (read/write locks)
//! - Deadlock detection

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use maharit_core::{ConcurrentGraph, EdgeId, Graph, NodeId, PropertyValue};
use thiserror::Error;

/// Transaction ID type
pub type TxId = u64;

/// Transaction errors
#[derive(Debug, Error, Clone, PartialEq)]
pub enum TransactionError {
    #[error("transaction {0} not found")]
    NotFound(TxId),

    #[error("transaction {0} already committed or rolled back")]
    AlreadyFinished(TxId),

    #[error("lock timeout: could not acquire lock on resource {0}")]
    LockTimeout(String),

    #[error("deadlock detected")]
    Deadlock,

    #[error("write conflict: resource {0} modified by another transaction")]
    WriteConflict(String),

    #[error("transaction {0} is read-only")]
    ReadOnly(TxId),
}

pub type Result<T> = std::result::Result<T, TransactionError>;

/// Lock mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Shared lock for reading
    Read,
    /// Exclusive lock for writing
    Write,
}

/// Transaction state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Active,
    Committed,
    RolledBack,
}

/// A change recorded during a transaction for rollback
#[derive(Debug, Clone)]
enum UndoRecord {
    CreateNode {
        node_id: NodeId,
    },
    DeleteNode {
        #[allow(dead_code)] // Kept for potential future use (e.g., ID restoration)
        node_id: NodeId,
        labels: Vec<String>,
        properties: Arc<HashMap<String, PropertyValue>>,
    },
    SetProperty {
        node_id: NodeId,
        key: String,
        old_value: Option<PropertyValue>,
    },
    CreateEdge {
        edge_id: EdgeId,
    },
    DeleteEdge {
        #[allow(dead_code)]
        edge_id: EdgeId,
        from: NodeId,
        to: NodeId,
        label: String,
        properties: Arc<HashMap<String, PropertyValue>>,
    },
    SetEdgeProperty {
        edge_id: EdgeId,
        key: String,
        old_value: Option<PropertyValue>,
    },
}

/// Lock information for a resource
#[derive(Debug)]
struct LockInfo {
    mode: LockMode,
    holders: HashSet<TxId>,
    waiting: VecDeque<(TxId, LockMode, Instant)>,
}

impl LockInfo {
    fn new() -> Self {
        Self {
            mode: LockMode::Read,
            holders: HashSet::new(),
            waiting: VecDeque::new(),
        }
    }

    fn can_grant(&self, mode: LockMode, tx_id: TxId) -> bool {
        if self.holders.is_empty() {
            return true;
        }

        // Already holding the lock
        if self.holders.contains(&tx_id) {
            // Can always keep read, upgrade to write only if sole holder
            return mode == LockMode::Read || self.holders.len() == 1;
        }

        match (self.mode, mode) {
            // Multiple readers allowed
            (LockMode::Read, LockMode::Read) => true,
            // Cannot read if writer holds
            (LockMode::Write, _) => false,
            // Cannot write if anyone holds
            (LockMode::Read, LockMode::Write) => false,
        }
    }
}

/// A transaction handle
#[derive(Debug)]
pub struct Transaction {
    pub id: TxId,
    pub state: TransactionState,
    pub read_only: bool,
    started_at: Instant,
    undo_log: Vec<UndoRecord>,
    held_locks: HashSet<String>,
}

impl Transaction {
    fn new(id: TxId, read_only: bool) -> Self {
        Self {
            id,
            state: TransactionState::Active,
            read_only,
            started_at: Instant::now(),
            undo_log: Vec::new(),
            held_locks: HashSet::new(),
        }
    }

    /// Get the duration since transaction started
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

/// Transaction manager
pub struct TransactionManager {
    next_tx_id: AtomicU64,
    transactions: RwLock<HashMap<TxId, Arc<Mutex<Transaction>>>>,
    locks: RwLock<HashMap<String, LockInfo>>,
    lock_timeout: Duration,
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            next_tx_id: AtomicU64::new(1),
            transactions: RwLock::new(HashMap::new()),
            locks: RwLock::new(HashMap::new()),
            lock_timeout: Duration::from_secs(30),
        }
    }

    /// Set the lock timeout duration
    pub fn set_lock_timeout(&mut self, timeout: Duration) {
        self.lock_timeout = timeout;
    }

    /// Begin a new transaction
    pub fn begin(&self) -> TxId {
        self.begin_with_options(false)
    }

    /// Begin a read-only transaction
    pub fn begin_read_only(&self) -> TxId {
        self.begin_with_options(true)
    }

    fn begin_with_options(&self, read_only: bool) -> TxId {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::SeqCst);
        let tx = Transaction::new(tx_id, read_only);

        let mut txs = self.transactions.write().unwrap();
        txs.insert(tx_id, Arc::new(Mutex::new(tx)));

        tx_id
    }

    /// Commit a transaction
    pub fn commit(&self, tx_id: TxId) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        if tx.state != TransactionState::Active {
            return Err(TransactionError::AlreadyFinished(tx_id));
        }

        // Release all locks
        self.release_all_locks(tx_id, &tx.held_locks);

        // Clear undo log (changes are now permanent)
        tx.undo_log.clear();
        tx.state = TransactionState::Committed;

        Ok(())
    }

    /// Rollback a transaction
    pub fn rollback(&self, tx_id: TxId, graph: &mut Graph) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        if tx.state != TransactionState::Active {
            return Err(TransactionError::AlreadyFinished(tx_id));
        }

        // Apply undo records in reverse order
        for record in tx.undo_log.iter().rev() {
            match record {
                UndoRecord::CreateNode { node_id } => {
                    graph.delete_node(*node_id);
                }
                UndoRecord::DeleteNode {
                    node_id: _,
                    labels,
                    properties,
                } => {
                    let new_id = graph.create_node_with_labels(labels.clone());
                    if let Some(node) = graph.get_node_mut(new_id) {
                        for (key, value) in properties.iter() {
                            node.set_property(key.clone(), value.clone());
                        }
                    }
                }
                UndoRecord::SetProperty {
                    node_id,
                    key,
                    old_value,
                } => {
                    if let Some(node) = graph.get_node_mut(*node_id) {
                        match old_value {
                            Some(value) => node.set_property(key.clone(), value.clone()),
                            None => {
                                node.remove_property(key);
                            }
                        }
                    }
                }
                UndoRecord::CreateEdge { edge_id } => {
                    graph.delete_edge(*edge_id);
                }
                UndoRecord::DeleteEdge {
                    edge_id: _,
                    from,
                    to,
                    label,
                    properties,
                } => {
                    if let Ok(new_id) = graph.create_edge(*from, *to, label.clone())
                        && let Some(edge) = graph.get_edge_mut(new_id) {
                            for (k, v) in properties.iter() {
                                edge.set_property(k.clone(), v.clone());
                            }
                        }
                }
                UndoRecord::SetEdgeProperty {
                    edge_id,
                    key,
                    old_value,
                } => {
                    if let Some(edge) = graph.get_edge_mut(*edge_id) {
                        match old_value {
                            Some(value) => edge.set_property(key.clone(), value.clone()),
                            None => {
                                edge.remove_property(key);
                            }
                        }
                    }
                }
            }
        }

        // Release all locks
        self.release_all_locks(tx_id, &tx.held_locks);

        tx.undo_log.clear();
        tx.state = TransactionState::RolledBack;

        Ok(())
    }

    /// Rollback a transaction against a `ConcurrentGraph`.
    ///
    /// Unlike [`rollback`] which requires `&mut Graph`, this method takes a
    /// shared `&ConcurrentGraph` because DashMap provides interior mutability.
    pub fn rollback_concurrent(&self, tx_id: TxId, graph: &ConcurrentGraph) -> Result<()> {
        use std::sync::Arc as StdArc;
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        if tx.state != TransactionState::Active {
            return Err(TransactionError::AlreadyFinished(tx_id));
        }

        // Apply undo records in reverse order.
        for record in tx.undo_log.iter().rev() {
            match record {
                UndoRecord::CreateNode { node_id } => {
                    graph.delete_node(*node_id);
                }
                UndoRecord::DeleteNode { node_id: _, labels, properties } => {
                    let new_id = graph.create_node_with_labels(labels.clone());
                    for (key, value) in properties.iter() {
                        graph.set_node_property(new_id, key, value.clone());
                    }
                }
                UndoRecord::SetProperty { node_id, key, old_value } => match old_value {
                    Some(value) => graph.set_node_property(*node_id, key, value.clone()),
                    None => {
                        graph.with_node_mut(*node_id, |n| {
                            StdArc::make_mut(&mut n.properties).remove(key.as_str());
                        });
                    }
                },
                UndoRecord::CreateEdge { edge_id } => {
                    graph.delete_edge(*edge_id);
                }
                UndoRecord::DeleteEdge { edge_id: _, from, to, label, properties } => {
                    if let Ok(new_id) = graph.create_edge(*from, *to, label.clone()) {
                        for (k, v) in properties.iter() {
                            graph.set_edge_property(new_id, k, v.clone());
                        }
                    }
                }
                UndoRecord::SetEdgeProperty { edge_id, key, old_value } => match old_value {
                    Some(value) => graph.set_edge_property(*edge_id, key, value.clone()),
                    None => {
                        graph.with_edge_mut(*edge_id, |e| {
                            StdArc::make_mut(&mut e.properties).remove(key.as_str());
                        });
                    }
                },
            }
        }

        // Release all locks held by this transaction.
        self.release_all_locks(tx_id, &tx.held_locks);
        tx.undo_log.clear();
        tx.state = TransactionState::RolledBack;

        Ok(())
    }

    /// Create a node within a transaction
    pub fn create_node(&self, tx_id: TxId, graph: &mut Graph, label: &str) -> Result<NodeId> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        if tx.read_only {
            return Err(TransactionError::ReadOnly(tx_id));
        }

        if tx.state != TransactionState::Active {
            return Err(TransactionError::AlreadyFinished(tx_id));
        }

        let node_id = graph.create_node(label);

        // Record undo action
        tx.undo_log.push(UndoRecord::CreateNode { node_id });

        Ok(node_id)
    }

    /// Delete a node within a transaction
    pub fn delete_node(&self, tx_id: TxId, graph: &mut Graph, node_id: NodeId) -> Result<bool> {
        {
            let tx_arc = self.get_transaction(tx_id)?;
            let tx = tx_arc.lock().unwrap();

            if tx.read_only {
                return Err(TransactionError::ReadOnly(tx_id));
            }

            if tx.state != TransactionState::Active {
                return Err(TransactionError::AlreadyFinished(tx_id));
            }
        }

        // Acquire write lock on the node
        let resource = format!("node:{}", node_id);
        self.acquire_lock(tx_id, &resource, LockMode::Write)?;

        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        // Save node state for undo
        if let Some(node) = graph.get_node(node_id) {
            let labels = node.labels.clone();
            let properties = node.properties.clone();

            tx.undo_log.push(UndoRecord::DeleteNode {
                node_id,
                labels,
                properties,
            });
        }

        Ok(graph.delete_node(node_id).is_some())
    }

    /// Set a node property within a transaction
    pub fn set_property(
        &self,
        tx_id: TxId,
        graph: &mut Graph,
        node_id: NodeId,
        key: String,
        value: PropertyValue,
    ) -> Result<()> {
        {
            let tx_arc = self.get_transaction(tx_id)?;
            let tx = tx_arc.lock().unwrap();

            if tx.read_only {
                return Err(TransactionError::ReadOnly(tx_id));
            }

            if tx.state != TransactionState::Active {
                return Err(TransactionError::AlreadyFinished(tx_id));
            }
        }

        // Acquire write lock
        let resource = format!("node:{}", node_id);
        self.acquire_lock(tx_id, &resource, LockMode::Write)?;

        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();

        // Save old value for undo
        let old_value = graph
            .get_node(node_id)
            .and_then(|n| n.properties.get(&key).cloned());

        tx.undo_log.push(UndoRecord::SetProperty {
            node_id,
            key: key.clone(),
            old_value,
        });

        if let Some(node) = graph.get_node_mut(node_id) {
            node.set_property(key, value);
        }

        Ok(())
    }

    // ── External undo-log recording API ──────────────────────────────────────
    // These methods allow the server layer (tcp_server) to record undo entries
    // for operations performed via the Executor, which is unaware of transactions.

    /// Record that a node was created (undo = delete it on rollback)
    pub fn record_node_created(&self, tx_id: TxId, node_id: NodeId) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::CreateNode { node_id });
        }
        Ok(())
    }

    /// Record that a node was deleted (undo = restore it on rollback)
    pub fn record_node_deleted(
        &self,
        tx_id: TxId,
        node_id: NodeId,
        labels: Vec<String>,
        properties: Arc<HashMap<String, PropertyValue>>,
    ) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::DeleteNode {
                node_id,
                labels,
                properties,
            });
        }
        Ok(())
    }

    /// Record that a node property was changed (undo = restore old value on rollback)
    pub fn record_property_changed(
        &self,
        tx_id: TxId,
        node_id: NodeId,
        key: String,
        old_value: Option<PropertyValue>,
    ) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::SetProperty {
                node_id,
                key,
                old_value,
            });
        }
        Ok(())
    }

    /// Record that an edge was created (undo = delete it on rollback)
    pub fn record_edge_created(&self, tx_id: TxId, edge_id: EdgeId) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::CreateEdge { edge_id });
        }
        Ok(())
    }

    /// Record that an edge was deleted (undo = restore it on rollback)
    pub fn record_edge_deleted(
        &self,
        tx_id: TxId,
        edge_id: EdgeId,
        from: NodeId,
        to: NodeId,
        label: String,
        properties: Arc<HashMap<String, PropertyValue>>,
    ) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::DeleteEdge {
                edge_id,
                from,
                to,
                label,
                properties,
            });
        }
        Ok(())
    }

    /// Record that an edge property was changed (undo = restore old value on rollback)
    pub fn record_edge_property_changed(
        &self,
        tx_id: TxId,
        edge_id: EdgeId,
        key: String,
        old_value: Option<PropertyValue>,
    ) -> Result<()> {
        let tx_arc = self.get_transaction(tx_id)?;
        let mut tx = tx_arc.lock().unwrap();
        if tx.state == TransactionState::Active {
            tx.undo_log.push(UndoRecord::SetEdgeProperty {
                edge_id,
                key,
                old_value,
            });
        }
        Ok(())
    }

    /// Acquire a lock on a resource
    pub fn acquire_lock(&self, tx_id: TxId, resource: &str, mode: LockMode) -> Result<()> {
        let start = Instant::now();

        loop {
            {
                let mut locks = self.locks.write().unwrap();

                // Check for deadlock first (before modifying)
                if self.would_cause_deadlock_internal(tx_id, resource, &locks) {
                    return Err(TransactionError::Deadlock);
                }

                let lock_info = locks
                    .entry(resource.to_string())
                    .or_insert_with(LockInfo::new);

                if lock_info.can_grant(mode, tx_id) {
                    // Grant the lock
                    if mode == LockMode::Write || lock_info.holders.is_empty() {
                        lock_info.mode = mode;
                    }
                    lock_info.holders.insert(tx_id);

                    // Record lock in transaction
                    if let Ok(tx_arc) = self.get_transaction(tx_id) {
                        let mut tx = tx_arc.lock().unwrap();
                        tx.held_locks.insert(resource.to_string());
                    }

                    return Ok(());
                }

                // Add to waiting queue if not already there
                if !lock_info.waiting.iter().any(|(id, _, _)| *id == tx_id) {
                    lock_info.waiting.push_back((tx_id, mode, Instant::now()));
                }
            }

            // Check timeout
            if start.elapsed() > self.lock_timeout {
                // Remove from waiting queue
                let mut locks = self.locks.write().unwrap();
                if let Some(lock_info) = locks.get_mut(resource) {
                    lock_info.waiting.retain(|(id, _, _)| *id != tx_id);
                }
                return Err(TransactionError::LockTimeout(resource.to_string()));
            }

            // Wait a bit before retrying
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Release a specific lock
    pub fn release_lock(&self, tx_id: TxId, resource: &str) {
        let mut locks = self.locks.write().unwrap();

        if let Some(lock_info) = locks.get_mut(resource) {
            lock_info.holders.remove(&tx_id);

            // Grant lock to next waiter if possible
            if lock_info.holders.is_empty()
                && let Some((next_tx, next_mode, _)) = lock_info.waiting.pop_front()
            {
                lock_info.mode = next_mode;
                lock_info.holders.insert(next_tx);
            }
        }

        // Remove from transaction's held locks
        if let Ok(tx_arc) = self.get_transaction(tx_id) {
            let mut tx = tx_arc.lock().unwrap();
            tx.held_locks.remove(resource);
        }
    }

    /// Release all locks held by a transaction
    fn release_all_locks(&self, tx_id: TxId, held_locks: &HashSet<String>) {
        let mut locks = self.locks.write().unwrap();

        for resource in held_locks {
            if let Some(lock_info) = locks.get_mut(resource) {
                lock_info.holders.remove(&tx_id);

                // Grant to next waiter
                if lock_info.holders.is_empty()
                    && let Some((next_tx, next_mode, _)) = lock_info.waiting.pop_front()
                {
                    lock_info.mode = next_mode;
                    lock_info.holders.insert(next_tx);
                }
            }
        }
    }

    /// Check if granting a lock would cause a deadlock (simple wait-for graph check)
    fn would_cause_deadlock_internal(
        &self,
        tx_id: TxId,
        resource: &str,
        locks: &HashMap<String, LockInfo>,
    ) -> bool {
        // Build wait-for edges: tx_id waits for all holders of the resource
        let Some(lock_info) = locks.get(resource) else {
            return false;
        };

        let mut visited = HashSet::new();
        let mut stack = Vec::new();

        // Start with transactions that hold the lock we want
        for holder in &lock_info.holders {
            if *holder != tx_id {
                stack.push(*holder);
            }
        }

        while let Some(current) = stack.pop() {
            if current == tx_id {
                // Found a cycle back to our transaction
                return true;
            }

            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            // Find what this transaction is waiting for
            for (res, info) in locks {
                if res == resource {
                    continue;
                }
                if info.waiting.iter().any(|(id, _, _)| *id == current) {
                    // Current is waiting for holders of this resource
                    for holder in &info.holders {
                        if !visited.contains(holder) {
                            stack.push(*holder);
                        }
                    }
                }
            }
        }

        false
    }

    /// Get a transaction by ID
    fn get_transaction(&self, tx_id: TxId) -> Result<Arc<Mutex<Transaction>>> {
        let txs = self.transactions.read().unwrap();
        txs.get(&tx_id)
            .cloned()
            .ok_or(TransactionError::NotFound(tx_id))
    }

    /// Get the current transaction count
    pub fn active_transaction_count(&self) -> usize {
        let txs = self.transactions.read().unwrap();
        txs.values()
            .filter(|tx| {
                let tx = tx.lock().unwrap();
                tx.state == TransactionState::Active
            })
            .count()
    }

    /// List all active transactions
    pub fn list_active_transactions(&self) -> Vec<TxId> {
        let txs = self.transactions.read().unwrap();
        txs.iter()
            .filter_map(|(id, tx)| {
                let tx = tx.lock().unwrap();
                if tx.state == TransactionState::Active {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_core::GraphBackend;

    #[test]
    fn test_begin_commit() {
        let tm = TransactionManager::new();
        let tx_id = tm.begin();

        assert!(tm.active_transaction_count() == 1);
        assert!(tm.commit(tx_id).is_ok());
        assert!(tm.active_transaction_count() == 0);
    }

    #[test]
    fn test_begin_rollback() {
        let tm = TransactionManager::new();
        let mut graph = Graph::new();

        let tx_id = tm.begin();
        let node_id = tm.create_node(tx_id, &mut graph, "Person").unwrap();

        assert_eq!(graph.node_count(), 1);

        tm.rollback(tx_id, &mut graph).unwrap();

        assert_eq!(graph.node_count(), 0);
        assert!(graph.get_node(node_id).is_none());
    }

    #[test]
    fn test_rollback_property_change() {
        let tm = TransactionManager::new();
        let mut graph = Graph::new();

        // Create a node outside transaction
        let node_id = graph.create_node("Person");
        graph
            .get_node_mut(node_id)
            .unwrap()
            .set_property("name", PropertyValue::String("Alice".to_string()));

        // Modify in transaction
        let tx_id = tm.begin();
        tm.set_property(
            tx_id,
            &mut graph,
            node_id,
            "name".to_string(),
            PropertyValue::String("Bob".to_string()),
        )
        .unwrap();

        assert_eq!(
            graph.get_node(node_id).unwrap().properties.get("name"),
            Some(&PropertyValue::String("Bob".to_string()))
        );

        // Rollback
        tm.rollback(tx_id, &mut graph).unwrap();

        assert_eq!(
            graph.get_node(node_id).unwrap().properties.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_read_only_transaction() {
        let tm = TransactionManager::new();
        let mut graph = Graph::new();

        let tx_id = tm.begin_read_only();
        let result = tm.create_node(tx_id, &mut graph, "Person");

        assert!(matches!(result, Err(TransactionError::ReadOnly(_))));
    }

    #[test]
    fn test_lock_acquisition() {
        let tm = TransactionManager::new();

        let tx1 = tm.begin();
        let tx2 = tm.begin();

        // tx1 acquires read lock
        assert!(tm.acquire_lock(tx1, "resource1", LockMode::Read).is_ok());

        // tx2 can also acquire read lock
        assert!(tm.acquire_lock(tx2, "resource1", LockMode::Read).is_ok());

        tm.commit(tx1).unwrap();
        tm.commit(tx2).unwrap();
    }

    #[test]
    fn test_write_lock_exclusion() {
        let tm = TransactionManager::new();

        let tx1 = tm.begin();

        // tx1 acquires write lock
        assert!(tm.acquire_lock(tx1, "resource1", LockMode::Write).is_ok());

        // Commit releases the lock
        tm.commit(tx1).unwrap();

        // Now another transaction can acquire
        let tx2 = tm.begin();
        assert!(tm.acquire_lock(tx2, "resource1", LockMode::Write).is_ok());
        tm.commit(tx2).unwrap();
    }

    #[test]
    fn test_double_commit_fails() {
        let tm = TransactionManager::new();
        let tx_id = tm.begin();

        assert!(tm.commit(tx_id).is_ok());
        assert!(matches!(
            tm.commit(tx_id),
            Err(TransactionError::AlreadyFinished(_))
        ));
    }

    #[test]
    fn test_rollback_concurrent_create_node() {
        use maharit_core::ConcurrentGraph;

        let tm = TransactionManager::new();
        let graph = ConcurrentGraph::new();

        let tx_id = tm.begin();
        let node_id = graph.create_node_with_labels(vec!["Person".to_string()]);
        tm.record_node_created(tx_id, node_id).unwrap();

        assert_eq!(graph.node_count(), 1);

        tm.rollback_concurrent(tx_id, &graph).unwrap();

        assert_eq!(graph.node_count(), 0);
        assert!(graph.get_node(node_id).is_none());
    }

    #[test]
    fn test_rollback_concurrent_delete_node() {
        use std::collections::HashMap;
        use std::sync::Arc;
        use maharit_core::{ConcurrentGraph, PropertyValue};

        let tm = TransactionManager::new();
        let graph = ConcurrentGraph::new();

        let node_id = graph.create_node_with_labels(vec!["Person".to_string()]);
        graph.set_node_property(node_id, "name", PropertyValue::String("Alice".to_string()));

        // Simulate deleting the node in a transaction
        let tx_id = tm.begin();
        let labels = vec!["Person".to_string()];
        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String("Alice".to_string()));
        tm.record_node_deleted(tx_id, node_id, labels, Arc::new(props)).unwrap();
        graph.delete_node(node_id);

        assert_eq!(graph.node_count(), 0);

        // Rollback should restore the node (with a new ID since ConcurrentGraph doesn't reuse IDs)
        tm.rollback_concurrent(tx_id, &graph).unwrap();

        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_rollback_concurrent_property_change() {
        use maharit_core::{ConcurrentGraph, PropertyValue};

        let tm = TransactionManager::new();
        let graph = ConcurrentGraph::new();

        let node_id = graph.create_node_with_labels(vec!["Person".to_string()]);
        graph.set_node_property(node_id, "name", PropertyValue::String("Alice".to_string()));

        // Modify in transaction
        let tx_id = tm.begin();
        let old = Some(PropertyValue::String("Alice".to_string()));
        tm.record_property_changed(tx_id, node_id, "name".to_string(), old).unwrap();
        graph.set_node_property(node_id, "name", PropertyValue::String("Bob".to_string()));

        let n = graph.get_node(node_id).unwrap();
        assert_eq!(
            n.properties.get("name"),
            Some(&PropertyValue::String("Bob".to_string()))
        );

        // Rollback
        tm.rollback_concurrent(tx_id, &graph).unwrap();

        let n = graph.get_node(node_id).unwrap();
        assert_eq!(
            n.properties.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_rollback_concurrent_already_finished() {
        use maharit_core::ConcurrentGraph;

        let tm = TransactionManager::new();
        let graph = ConcurrentGraph::new();

        let tx_id = tm.begin();
        tm.commit(tx_id).unwrap();

        assert!(matches!(
            tm.rollback_concurrent(tx_id, &graph),
            Err(TransactionError::AlreadyFinished(_))
        ));
    }

    #[test]
    fn test_multiple_transactions() {
        let tm = TransactionManager::new();

        let tx1 = tm.begin();
        let tx2 = tm.begin();
        let tx3 = tm.begin();

        assert_eq!(tm.active_transaction_count(), 3);

        tm.commit(tx1).unwrap();
        assert_eq!(tm.active_transaction_count(), 2);

        let mut graph = Graph::new();
        tm.rollback(tx2, &mut graph).unwrap();
        assert_eq!(tm.active_transaction_count(), 1);

        tm.commit(tx3).unwrap();
        assert_eq!(tm.active_transaction_count(), 0);
    }
}
