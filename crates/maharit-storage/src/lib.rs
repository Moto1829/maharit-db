pub mod backup;
pub mod mvcc;
mod persistence;
mod transaction;
mod wal;

use maharit_core::Graph;

pub use backup::{
    Backup, BackupCallback, BackupError, BackupMetadata, BackupOptions, BackupScheduler,
    CompressionType, IncrementalBackupMetadata,
};
pub use mvcc::{MvccManager, MvccSnapshot, NodeData, NodeVersion, PropertyValue as MvccPropertyValue, VersionedNode};
pub use persistence::{PersistenceError, PersistentStorage};
pub use transaction::{
    LockMode, Transaction, TransactionError, TransactionManager, TransactionState, TxId,
};
pub use wal::{LogRecord, Lsn, RecordPayload, RecordType, Wal, WalError};

/// インメモリストレージエンジン
pub struct InMemoryStorage {
    graph: Graph,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
        }
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}
