mod persistence;
mod wal;

use maharit_core::Graph;

pub use persistence::{PersistenceError, PersistentStorage};
pub use wal::{Lsn, LogRecord, RecordPayload, RecordType, Wal, WalError};

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
