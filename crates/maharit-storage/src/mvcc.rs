//! Multi-Version Concurrency Control (MVCC)
//!
//! Provides snapshot isolation for concurrent graph transactions.
//!
//! ## Visibility rule
//!
//! A [`NodeVersion`] `v` is visible to a [`MvccSnapshot`] `s` when **all**
//! of the following hold:
//!
//! 1. `v.created_at <= s.snapshot_id`  – the version was stamped before the
//!    snapshot was taken.
//! 2. `v.creator_txn ∉ s.active_txns` – the creator had already committed
//!    when the snapshot was taken.
//! 3. The version has not been logically deleted in a way visible to `s`:
//!    * `v.deleted_at` is `None`, **or**
//!    * `v.deleted_at > s.snapshot_id` (deletion is in the future), **or**
//!    * `v.deleter_txn` is in `s.active_txns` (the deleter had not yet
//!      committed when the snapshot was taken).
//!
//! ## Transaction lifecycle
//!
//! ```text
//! begin_transaction() -> txn_id
//! write_node(node_id, data, txn_id)
//! delete_node(node_id, txn_id)
//! commit(txn_id)   -- or rollback(txn_id)
//! gc()             -- periodically clean up old versions
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ─── Value type ────────────────────────────────────────────────────────────

/// A property value stored in a node snapshot.
///
/// Kept independent from `maharit_core::PropertyValue` so the MVCC layer
/// does not depend on the graph crate.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

// ─── Node data ─────────────────────────────────────────────────────────────

/// A snapshot of a node's label and properties at a specific logical time.
#[derive(Debug, Clone)]
pub struct NodeData {
    pub label: String,
    pub properties: HashMap<String, PropertyValue>,
}

// ─── Version ───────────────────────────────────────────────────────────────

/// One entry in a node's version chain.
#[derive(Debug, Clone)]
pub struct NodeVersion {
    /// Transaction that created this version.
    pub txn_id: u64,
    /// Logical clock value when this version was written.
    pub created_at: u64,
    /// Logical clock value when this version was logically deleted
    /// (`None` = still live).
    pub deleted_at: Option<u64>,
    /// Transaction that issued the logical delete (if any).
    pub deleter_txn: Option<u64>,
    /// The node data captured by this version.
    pub data: NodeData,
}

/// All versions of a single node, stored as an append-only chain ordered by
/// `created_at`.
#[derive(Debug, Default)]
pub struct VersionedNode {
    pub node_id: u64,
    pub versions: Vec<NodeVersion>,
}

// ─── Snapshot ──────────────────────────────────────────────────────────────

/// A read-consistent point-in-time view of the database.
///
/// Created by [`MvccManager::begin_snapshot`] and passed to every read
/// operation.
#[derive(Debug, Clone)]
pub struct MvccSnapshot {
    /// The logical clock value at the moment the snapshot was taken.
    pub snapshot_id: u64,
    /// Transaction IDs that were still active (uncommitted) at snapshot time.
    /// Their writes and deletes are invisible to this snapshot.
    pub active_txns: HashSet<u64>,
}

impl MvccSnapshot {
    /// Return `true` when `version` satisfies all visibility rules.
    fn is_visible(&self, version: &NodeVersion) -> bool {
        // Rule 1: version must have been created at or before the snapshot.
        if version.created_at > self.snapshot_id {
            return false;
        }

        // Rule 2: the creator must have committed before the snapshot.
        if self.active_txns.contains(&version.txn_id) {
            return false;
        }

        // Rule 3: check deletion visibility.
        match (version.deleted_at, version.deleter_txn) {
            (None, _) => {
                // Not deleted → visible.
                true
            }
            (Some(del_ts), Some(deleter)) => {
                // Deletion is invisible when:
                //   a) the deleter was still active at snapshot time, OR
                //   b) the deletion timestamp is after the snapshot.
                self.active_txns.contains(&deleter) || del_ts > self.snapshot_id
            }
            (Some(del_ts), None) => {
                // No deleter recorded (shouldn't normally happen) – fall back
                // to timestamp comparison.
                del_ts > self.snapshot_id
            }
        }
    }
}

// ─── Manager ───────────────────────────────────────────────────────────────

/// Manages all versioned nodes and coordinates MVCC snapshots.
pub struct MvccManager {
    versioned_nodes: HashMap<u64, VersionedNode>,
    /// Monotonically increasing logical clock.
    clock: AtomicU64,
    /// Set of currently active (uncommitted) transaction IDs.
    active_transactions: Arc<Mutex<HashSet<u64>>>,
    /// Next transaction ID to issue.
    next_txn_id: AtomicU64,
}

impl Default for MvccManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MvccManager {
    /// Create a new, empty MVCC manager.
    pub fn new() -> Self {
        Self {
            versioned_nodes: HashMap::new(),
            clock: AtomicU64::new(1),
            active_transactions: Arc::new(Mutex::new(HashSet::new())),
            next_txn_id: AtomicU64::new(1),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Advance the logical clock and return the new value.
    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::SeqCst)
    }

    // ── Transaction lifecycle ─────────────────────────────────────────────

    /// Allocate a fresh transaction ID and mark it as active.
    ///
    /// The caller is responsible for eventually calling [`commit`] or
    /// [`rollback`].
    pub fn begin_transaction(&self) -> u64 {
        let txn_id = self.next_txn_id.fetch_add(1, Ordering::SeqCst);
        self.active_transactions.lock().unwrap().insert(txn_id);
        txn_id
    }

    /// Take a consistent snapshot of the database state.
    ///
    /// The snapshot captures:
    /// * The current logical clock value (`snapshot_id`).
    /// * The set of transactions that are currently active (`active_txns`).
    ///
    /// Pass the returned [`MvccSnapshot`] to [`read_node`] to obtain a
    /// consistent, isolated view.
    pub fn begin_snapshot(&self) -> MvccSnapshot {
        // Capture active transactions first, then the clock, so that a
        // transaction that starts *after* begin_snapshot() is called will
        // have a clock value > snapshot_id and will therefore be invisible.
        let active = self.active_transactions.lock().unwrap().clone();
        // Use fetch_add(0) to read without incrementing.
        let snapshot_id = self.clock.fetch_add(1, Ordering::SeqCst);
        MvccSnapshot {
            snapshot_id,
            active_txns: active,
        }
    }

    /// Read the most-recent version of `node_id` that is visible under
    /// `snapshot`.
    ///
    /// Returns `None` when the node does not exist or every version is
    /// invisible or logically deleted before the snapshot.
    pub fn read_node(&self, node_id: u64, snapshot: &MvccSnapshot) -> Option<&NodeData> {
        let versioned = self.versioned_nodes.get(&node_id)?;

        // Walk from newest to oldest; return the first visible version.
        versioned
            .versions
            .iter()
            .rev()
            .find(|v| snapshot.is_visible(v))
            .map(|v| &v.data)
    }

    /// Append a new version to `node_id`'s version chain.
    ///
    /// If the node has never been seen, a [`VersionedNode`] entry is created
    /// automatically.
    pub fn write_node(&mut self, node_id: u64, data: NodeData, txn_id: u64) {
        let ts = self.tick();
        let version = NodeVersion {
            txn_id,
            created_at: ts,
            deleted_at: None,
            deleter_txn: None,
            data,
        };
        self.versioned_nodes
            .entry(node_id)
            .or_insert_with(|| VersionedNode {
                node_id,
                versions: Vec::new(),
            })
            .versions
            .push(version);
    }

    /// Logically delete `node_id` on behalf of `txn_id`.
    ///
    /// Marks the most-recent non-deleted version of the node with a
    /// `deleted_at` timestamp and the `deleter_txn` so that snapshot
    /// visibility can be correctly determined.
    pub fn delete_node(&mut self, node_id: u64, txn_id: u64) {
        let ts = self.tick();
        if let Some(versioned) = self.versioned_nodes.get_mut(&node_id)
            && let Some(v) = versioned
                .versions
                .iter_mut()
                .rev()
                .find(|v| v.deleted_at.is_none())
        {
            v.deleted_at = Some(ts);
            v.deleter_txn = Some(txn_id);
        }
    }

    /// Commit `txn_id`: remove it from the active set so that future
    /// snapshots can see its writes.
    pub fn commit(&self, txn_id: u64) {
        self.active_transactions.lock().unwrap().remove(&txn_id);
    }

    /// Rollback `txn_id`: discard all versions it created and undo any
    /// logical deletes it issued, then remove it from the active set.
    pub fn rollback(&mut self, txn_id: u64) {
        for versioned in self.versioned_nodes.values_mut() {
            // Undo logical deletes issued by this transaction.
            for v in versioned.versions.iter_mut() {
                if v.deleter_txn == Some(txn_id) {
                    v.deleted_at = None;
                    v.deleter_txn = None;
                }
            }
            // Remove versions written by this transaction.
            versioned.versions.retain(|v| v.txn_id != txn_id);
        }
        self.active_transactions.lock().unwrap().remove(&txn_id);
    }

    // ── Garbage collection ────────────────────────────────────────────────

    /// Remove historical node versions that cannot be seen by any current or
    /// future snapshot.
    ///
    /// The GC watermark is the lowest logical clock value at which a new
    /// snapshot could be taken.  Any version whose `deleted_at` is below the
    /// watermark is invisible to all future snapshots and can be safely
    /// removed.  Additionally, an *older* non-deleted version is collectable
    /// when a *newer* committed version exists for the same node (the older
    /// version is superseded and no snapshot taken after the newer version's
    /// `created_at` would choose the older one).
    ///
    /// When there are no active transactions every deleted version (and every
    /// superseded non-deleted version) is eligible for collection.
    ///
    /// Returns the total number of version entries removed.
    pub fn gc(&mut self) -> usize {
        // GC watermark: the minimum snapshot_id any future snapshot could have.
        // * No active transactions → watermark = u64::MAX (collect everything
        //   that has been deleted or superseded).
        // * Active transactions present → watermark = current clock (be
        //   conservative; only collect versions deleted/superseded strictly
        //   before the current clock value).
        let watermark: u64 = {
            let active = self.active_transactions.lock().unwrap();
            if active.is_empty() {
                u64::MAX
            } else {
                self.clock.load(Ordering::SeqCst)
            }
        };

        let mut removed = 0usize;

        for versioned in self.versioned_nodes.values_mut() {
            // Work in chronological order.
            versioned.versions.sort_by_key(|v| v.created_at);

            let n = versioned.versions.len();
            let mut keep = vec![true; n];

            for (i, v) in versioned.versions.iter().enumerate() {
                if let Some(del_ts) = v.deleted_at {
                    // Version is logically deleted and the deletion is old
                    // enough that no future snapshot will see it as "live".
                    if del_ts < watermark {
                        keep[i] = false;
                    }
                } else {
                    // Non-deleted (live) version: it is superseded if a newer
                    // non-deleted version committed before the watermark exists.
                    let has_newer_committed = versioned.versions[i + 1..]
                        .iter()
                        .any(|nv| nv.deleted_at.is_none() && nv.created_at < watermark);
                    if has_newer_committed {
                        keep[i] = false;
                    }
                }
            }

            let mut iter = keep.iter();
            versioned.versions.retain(|_| *iter.next().unwrap());

            removed += n - versioned.versions.len();
        }

        // Remove empty version chains.
        self.versioned_nodes.retain(|_, vn| !vn.versions.is_empty());

        removed
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(label: &str) -> NodeData {
        NodeData {
            label: label.to_string(),
            properties: HashMap::new(),
        }
    }

    fn make_data_with_prop(label: &str, key: &str, value: &str) -> NodeData {
        let mut props = HashMap::new();
        props.insert(
            key.to_string(),
            PropertyValue::String(value.to_string()),
        );
        NodeData {
            label: label.to_string(),
            properties: props,
        }
    }

    // ── Basic read/write ──────────────────────────────────────────────────

    #[test]
    fn test_write_and_read_within_same_transaction() {
        let mut mgr = MvccManager::new();
        let txn = mgr.begin_transaction();
        mgr.write_node(1, make_data("Person"), txn);

        // Commit so subsequent snapshots can see it.
        mgr.commit(txn);

        let snap = mgr.begin_snapshot();
        let data = mgr.read_node(1, &snap);
        assert!(data.is_some(), "committed node should be readable");
        assert_eq!(data.unwrap().label, "Person");
    }

    #[test]
    fn test_uncommitted_write_invisible_to_other_snapshot() {
        let mut mgr = MvccManager::new();

        // Take snapshot BEFORE the write transaction starts.
        let snap = mgr.begin_snapshot();

        // Another transaction writes a node AFTER the snapshot was taken.
        let txn = mgr.begin_transaction();
        mgr.write_node(42, make_data("Secret"), txn);
        // txn is NOT committed.

        // The write must be invisible: its clock value > snap.snapshot_id.
        let data = mgr.read_node(42, &snap);
        assert!(
            data.is_none(),
            "write that happened after snapshot must not be visible"
        );
    }

    #[test]
    fn test_uncommitted_write_invisible_to_concurrent_snapshot() {
        let mut mgr = MvccManager::new();

        // Writer registers itself as active BEFORE the reader takes a snapshot.
        let writer_txn = mgr.begin_transaction();

        // Reader takes snapshot while writer is still active.
        let snap = mgr.begin_snapshot();

        mgr.write_node(10, make_data("Hidden"), writer_txn);
        // writer still not committed.

        // The writer is in snap.active_txns, so its version is invisible.
        let data = mgr.read_node(10, &snap);
        assert!(
            data.is_none(),
            "concurrent uncommitted write must be invisible to snapshot taken while txn was active"
        );
    }

    #[test]
    fn test_committed_write_visible_to_new_snapshot() {
        let mut mgr = MvccManager::new();

        let txn = mgr.begin_transaction();
        mgr.write_node(7, make_data_with_prop("City", "name", "Tokyo"), txn);
        mgr.commit(txn);

        // Snapshot taken AFTER commit.
        let snap = mgr.begin_snapshot();
        let data = mgr.read_node(7, &snap);

        assert!(data.is_some());
        assert_eq!(
            data.unwrap().properties.get("name"),
            Some(&PropertyValue::String("Tokyo".to_string()))
        );
    }

    // ── Rollback ──────────────────────────────────────────────────────────

    #[test]
    fn test_rollback_removes_versions() {
        let mut mgr = MvccManager::new();

        let txn = mgr.begin_transaction();
        mgr.write_node(99, make_data("Ephemeral"), txn);

        // Rollback before committing.
        mgr.rollback(txn);

        let snap = mgr.begin_snapshot();
        assert!(
            mgr.read_node(99, &snap).is_none(),
            "rolled-back node must be invisible"
        );
    }

    #[test]
    fn test_rollback_preserves_previous_version() {
        let mut mgr = MvccManager::new();

        // Commit an initial version.
        let txn1 = mgr.begin_transaction();
        mgr.write_node(5, make_data_with_prop("Node", "v", "v1"), txn1);
        mgr.commit(txn1);

        // A second transaction updates the node.
        let txn2 = mgr.begin_transaction();
        mgr.write_node(5, make_data_with_prop("Node", "v", "v2"), txn2);

        // Rollback txn2.
        mgr.rollback(txn2);

        // The original version should still be visible.
        let snap = mgr.begin_snapshot();
        let data = mgr
            .read_node(5, &snap)
            .expect("original version must survive rollback");
        assert_eq!(
            data.properties.get("v"),
            Some(&PropertyValue::String("v1".to_string()))
        );
    }

    // ── Delete ────────────────────────────────────────────────────────────

    #[test]
    fn test_delete_marks_node_invisible() {
        let mut mgr = MvccManager::new();

        let txn1 = mgr.begin_transaction();
        mgr.write_node(3, make_data("Mortal"), txn1);
        mgr.commit(txn1);

        // Snapshot before deletion.
        let snap_before = mgr.begin_snapshot();
        assert!(
            mgr.read_node(3, &snap_before).is_some(),
            "node should be visible before deletion"
        );

        let txn2 = mgr.begin_transaction();
        mgr.delete_node(3, txn2);
        mgr.commit(txn2);

        // Snapshot after deletion.
        let snap_after = mgr.begin_snapshot();
        assert!(
            mgr.read_node(3, &snap_after).is_none(),
            "deleted node should not be visible after commit"
        );

        // The pre-deletion snapshot still sees the old version.
        assert!(
            mgr.read_node(3, &snap_before).is_some(),
            "pre-deletion snapshot must still see the node"
        );
    }

    // ── Garbage collection ────────────────────────────────────────────────

    #[test]
    fn test_gc_removes_deleted_versions() {
        let mut mgr = MvccManager::new();

        // Write and commit a version.
        let txn1 = mgr.begin_transaction();
        mgr.write_node(20, make_data("Old"), txn1);
        mgr.commit(txn1);

        // Write a second (updated) version and commit.
        let txn2 = mgr.begin_transaction();
        mgr.write_node(20, make_data("New"), txn2);
        mgr.commit(txn2);

        // Delete the node and commit.
        let txn3 = mgr.begin_transaction();
        mgr.delete_node(20, txn3);
        mgr.commit(txn3);

        // No active transactions → GC should collect old deleted versions.
        let removed = mgr.gc();
        assert!(removed > 0, "GC should have removed at least one old version");
    }

    #[test]
    fn test_gc_does_not_remove_live_versions() {
        let mut mgr = MvccManager::new();

        let txn = mgr.begin_transaction();
        mgr.write_node(30, make_data("Alive"), txn);
        mgr.commit(txn);

        let removed = mgr.gc();
        assert_eq!(removed, 0, "GC must not remove live (non-deleted) versions");

        // Node is still readable after GC.
        let snap = mgr.begin_snapshot();
        assert!(mgr.read_node(30, &snap).is_some());
    }

    #[test]
    fn test_gc_returns_zero_for_empty_manager() {
        let mut mgr = MvccManager::new();
        let removed = mgr.gc();
        assert_eq!(removed, 0);
    }

    // ── Snapshot isolation ────────────────────────────────────────────────

    #[test]
    fn test_snapshot_isolation_sees_consistent_state() {
        let mut mgr = MvccManager::new();

        // Commit initial state.
        let txn1 = mgr.begin_transaction();
        mgr.write_node(100, make_data_with_prop("Item", "status", "initial"), txn1);
        mgr.commit(txn1);

        // Reader takes snapshot.
        let snap = mgr.begin_snapshot();

        // Writer updates the node after the snapshot.
        let txn2 = mgr.begin_transaction();
        mgr.write_node(
            100,
            make_data_with_prop("Item", "status", "updated"),
            txn2,
        );
        mgr.commit(txn2);

        // Reader's snapshot should still see the initial value.
        let data = mgr.read_node(100, &snap).expect("node must be visible");
        assert_eq!(
            data.properties.get("status"),
            Some(&PropertyValue::String("initial".to_string())),
            "snapshot should see state as of snapshot creation"
        );

        // A new snapshot sees the updated value.
        let snap2 = mgr.begin_snapshot();
        let data2 = mgr.read_node(100, &snap2).expect("node must be visible");
        assert_eq!(
            data2.properties.get("status"),
            Some(&PropertyValue::String("updated".to_string()))
        );
    }

    #[test]
    fn test_multiple_versions_read_latest_visible() {
        let mut mgr = MvccManager::new();

        let txn1 = mgr.begin_transaction();
        mgr.write_node(200, make_data_with_prop("X", "val", "1"), txn1);
        mgr.commit(txn1);

        let txn2 = mgr.begin_transaction();
        mgr.write_node(200, make_data_with_prop("X", "val", "2"), txn2);
        mgr.commit(txn2);

        let txn3 = mgr.begin_transaction();
        mgr.write_node(200, make_data_with_prop("X", "val", "3"), txn3);
        mgr.commit(txn3);

        let snap = mgr.begin_snapshot();
        let data = mgr.read_node(200, &snap).unwrap();
        assert_eq!(
            data.properties.get("val"),
            Some(&PropertyValue::String("3".to_string())),
            "should read the latest committed version"
        );
    }
}
