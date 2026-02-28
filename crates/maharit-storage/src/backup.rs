use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use maharit_core::{Graph, PropertyValue};
use thiserror::Error;
use tokio::sync::RwLock;

/// Backup file format magic number
const MAGIC: &[u8; 8] = b"MHRTBKUP";
/// Backup format version
const VERSION: u32 = 1;

/// Backup error types
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid magic number")]
    InvalidMagic,

    #[error("unsupported version: {0}")]
    UnsupportedVersion(u32),

    #[error("corrupted data: {0}")]
    CorruptedData(String),

    #[error("graph error: {0}")]
    Graph(#[from] maharit_core::GraphError),
}

pub type Result<T> = std::result::Result<T, BackupError>;

/// Options for creating a backup
#[derive(Debug, Clone)]
pub struct BackupOptions {
    /// Whether to compress the backup with gzip
    pub compressed: bool,
    /// Optional description of the backup
    pub description: String,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            compressed: false,
            description: String::new(),
        }
    }
}

impl BackupOptions {
    /// Create new backup options with compression enabled
    pub fn compressed() -> Self {
        Self {
            compressed: true,
            description: String::new(),
        }
    }

    /// Set the description for the backup
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

/// Metadata about a backup
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupMetadata {
    /// Backup format version
    pub version: u32,
    /// Unix timestamp when the backup was created
    pub timestamp: u64,
    /// Number of nodes in the original graph
    pub node_count: u64,
    /// Number of edges in the original graph
    pub edge_count: u64,
    /// Whether the backup data is compressed
    pub compressed: bool,
    /// Optional description of the backup
    pub description: String,
}

/// Callback invoked when a backup completes successfully.
///
/// Receives a reference to the `BackupMetadata` of the completed backup.
pub type BackupCallback = Box<dyn Fn(&BackupMetadata) + Send + Sync>;

/// Backup and restore functionality for graphs
pub struct Backup;

impl Backup {
    /// Create a backup of a graph with the specified options.
    ///
    /// The backup includes metadata (timestamp, node/edge counts, compression status)
    /// and the full graph data serialized in binary format.
    ///
    /// # Arguments
    /// * `graph` - The graph to backup
    /// * `path` - Path where the backup file will be written
    /// * `options` - Backup options (compression, description)
    ///
    /// # Returns
    /// The metadata of the created backup
    ///
    /// # Errors
    /// Returns `BackupError::Io` if the file cannot be written
    pub fn create(
        graph: &Graph,
        path: impl AsRef<Path>,
        options: &BackupOptions,
    ) -> Result<BackupMetadata> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Collect metadata
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_secs();

        let metadata = BackupMetadata {
            version: VERSION,
            timestamp,
            node_count: graph.node_count() as u64,
            edge_count: graph.edge_count() as u64,
            compressed: options.compressed,
            description: options.description.clone(),
        };

        // Write header
        writer.write_all(MAGIC)?;
        writer.write_all(&metadata.version.to_le_bytes())?;
        writer.write_all(&metadata.timestamp.to_le_bytes())?;
        writer.write_all(&metadata.node_count.to_le_bytes())?;
        writer.write_all(&metadata.edge_count.to_le_bytes())?;
        writer.write_all(&[if metadata.compressed { 1 } else { 0 }])?;
        Self::write_string(&mut writer, &metadata.description)?;

        // Serialize graph data to a buffer
        let graph_data = Self::serialize_graph(graph)?;

        // Write graph data (compressed or not)
        if options.compressed {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&graph_data)?;
            let compressed_data = encoder.finish()?;
            writer.write_all(&compressed_data)?;
        } else {
            writer.write_all(&graph_data)?;
        }

        writer.flush()?;
        Ok(metadata)
    }

    /// Create a backup asynchronously, taking a read lock on the shared graph.
    ///
    /// The read lock is acquired to create a serialized snapshot; the actual
    /// file write happens while the lock is held.  This allows other readers
    /// to continue concurrently while blocking only writers during the snapshot
    /// phase.
    ///
    /// # Arguments
    /// * `graph` - A reference-counted, async-RwLock–protected graph
    /// * `output_path` - Path where the backup file will be written
    /// * `options` - Backup options (compression, description)
    ///
    /// # Returns
    /// The metadata of the created backup
    ///
    /// # Errors
    /// Returns `BackupError::Io` if the file cannot be written
    pub async fn create_async(
        graph: Arc<RwLock<Graph>>,
        output_path: &str,
        options: &BackupOptions,
    ) -> Result<BackupMetadata> {
        // Acquire read lock and serialise the graph data while holding it
        let (graph_data, node_count, edge_count) = {
            let g = graph.read().await;
            let data = Self::serialize_graph(&g)?;
            (data, g.node_count() as u64, g.edge_count() as u64)
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_secs();

        let metadata = BackupMetadata {
            version: VERSION,
            timestamp,
            node_count,
            edge_count,
            compressed: options.compressed,
            description: options.description.clone(),
        };

        // Write file (no lock held here)
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(MAGIC)?;
        writer.write_all(&metadata.version.to_le_bytes())?;
        writer.write_all(&metadata.timestamp.to_le_bytes())?;
        writer.write_all(&metadata.node_count.to_le_bytes())?;
        writer.write_all(&metadata.edge_count.to_le_bytes())?;
        writer.write_all(&[if metadata.compressed { 1 } else { 0 }])?;
        Self::write_string(&mut writer, &metadata.description)?;

        if options.compressed {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&graph_data)?;
            let compressed_data = encoder.finish()?;
            writer.write_all(&compressed_data)?;
        } else {
            writer.write_all(&graph_data)?;
        }

        writer.flush()?;
        Ok(metadata)
    }

    /// Restore a graph from a backup file.
    ///
    /// Reads the backup metadata and graph data, automatically handling
    /// decompression if the backup was compressed.
    ///
    /// # Arguments
    /// * `path` - Path to the backup file
    ///
    /// # Returns
    /// The restored graph
    ///
    /// # Errors
    /// Returns errors if the file is invalid, corrupted, or cannot be read
    pub fn restore(path: impl AsRef<Path>) -> Result<Graph> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read and verify magic number
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BackupError::InvalidMagic);
        }

        // Read metadata
        let version = Self::read_u32(&mut reader)?;
        if version != VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }

        let _timestamp = Self::read_u64(&mut reader)?;
        let _node_count = Self::read_u64(&mut reader)?;
        let _edge_count = Self::read_u64(&mut reader)?;

        let mut compressed_flag = [0u8; 1];
        reader.read_exact(&mut compressed_flag)?;
        let compressed = compressed_flag[0] != 0;

        let _description = Self::read_string(&mut reader)?;

        // Read graph data (decompress if needed)
        let graph_data = if compressed {
            let mut decoder = GzDecoder::new(reader);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            decompressed
        } else {
            let mut data = Vec::new();
            reader.read_to_end(&mut data)?;
            data
        };

        // Deserialize graph
        Self::deserialize_graph(&graph_data)
    }

    /// Read metadata from a backup file without loading the graph data.
    ///
    /// This is useful for inspecting backups without the overhead of
    /// loading the entire graph.
    ///
    /// # Arguments
    /// * `path` - Path to the backup file
    ///
    /// # Returns
    /// The backup metadata
    ///
    /// # Errors
    /// Returns errors if the file is invalid or cannot be read
    pub fn metadata(path: impl AsRef<Path>) -> Result<BackupMetadata> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Read and verify magic number
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BackupError::InvalidMagic);
        }

        // Read metadata
        let version = Self::read_u32(&mut reader)?;
        if version != VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }

        let timestamp = Self::read_u64(&mut reader)?;
        let node_count = Self::read_u64(&mut reader)?;
        let edge_count = Self::read_u64(&mut reader)?;

        let mut compressed_flag = [0u8; 1];
        reader.read_exact(&mut compressed_flag)?;
        let compressed = compressed_flag[0] != 0;

        let description = Self::read_string(&mut reader)?;

        Ok(BackupMetadata {
            version,
            timestamp,
            node_count,
            edge_count,
            compressed,
            description,
        })
    }

    /// Verify that a backup file is valid and can be restored.
    ///
    /// This performs a full restoration to a temporary graph to ensure
    /// the backup is not corrupted and contains valid data.
    ///
    /// # Arguments
    /// * `path` - Path to the backup file
    ///
    /// # Returns
    /// `Ok(true)` if the backup is valid, `Ok(false)` should not occur,
    /// `Err` if the backup is invalid or corrupted
    ///
    /// # Errors
    /// Returns errors if the backup is corrupted or invalid
    pub fn verify(path: impl AsRef<Path>) -> Result<bool> {
        let metadata = Self::metadata(path.as_ref())?;
        let graph = Self::restore(path)?;

        // Verify counts match
        if graph.node_count() as u64 != metadata.node_count {
            return Err(BackupError::CorruptedData(format!(
                "node count mismatch: expected {}, got {}",
                metadata.node_count,
                graph.node_count()
            )));
        }

        if graph.edge_count() as u64 != metadata.edge_count {
            return Err(BackupError::CorruptedData(format!(
                "edge count mismatch: expected {}, got {}",
                metadata.edge_count,
                graph.edge_count()
            )));
        }

        Ok(true)
    }

    // ========== Graph serialization ==========

    fn serialize_graph(graph: &Graph) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();

        // Write node count and edge count
        let node_count = graph.node_count() as u64;
        let edge_count = graph.edge_count() as u64;
        buffer.write_all(&node_count.to_le_bytes())?;
        buffer.write_all(&edge_count.to_le_bytes())?;

        // Write nodes
        for node in graph.nodes() {
            Self::write_u64(&mut buffer, node.id)?;
            Self::write_string(&mut buffer, &node.label)?;
            Self::write_properties(&mut buffer, &node.properties)?;
        }

        // Write edges
        for edge in graph.edges() {
            Self::write_u64(&mut buffer, edge.id)?;
            Self::write_u64(&mut buffer, edge.from)?;
            Self::write_u64(&mut buffer, edge.to)?;
            Self::write_string(&mut buffer, &edge.label)?;
            Self::write_properties(&mut buffer, &edge.properties)?;
        }

        Ok(buffer)
    }

    fn deserialize_graph(data: &[u8]) -> Result<Graph> {
        let mut reader = std::io::Cursor::new(data);

        // Read node count and edge count
        let node_count = Self::read_u64(&mut reader)?;
        let edge_count = Self::read_u64(&mut reader)?;

        let mut graph = Graph::new();
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        // Read nodes
        for _ in 0..node_count {
            let old_id = Self::read_u64(&mut reader)?;
            let label = Self::read_string(&mut reader)?;
            let properties = Self::read_properties(&mut reader)?;

            let new_id = graph.create_node(&label);
            id_map.insert(old_id, new_id);

            if let Some(node) = graph.get_node_mut(new_id) {
                for (key, value) in properties {
                    node.set_property(key, value);
                }
            }
        }

        // Read edges
        for _ in 0..edge_count {
            let _old_id = Self::read_u64(&mut reader)?;
            let old_from = Self::read_u64(&mut reader)?;
            let old_to = Self::read_u64(&mut reader)?;
            let label = Self::read_string(&mut reader)?;
            let properties = Self::read_properties(&mut reader)?;

            let from = *id_map.get(&old_from).ok_or_else(|| {
                BackupError::CorruptedData(format!("unknown node id: {}", old_from))
            })?;
            let to = *id_map.get(&old_to).ok_or_else(|| {
                BackupError::CorruptedData(format!("unknown node id: {}", old_to))
            })?;

            let edge_id = graph.create_edge(from, to, &label)?;

            if let Some(edge) = graph.get_edge_mut(edge_id) {
                for (key, value) in properties {
                    edge.set_property(key, value);
                }
            }
        }

        Ok(graph)
    }

    // ========== Writer helpers ==========

    fn write_u64<W: Write>(writer: &mut W, value: u64) -> Result<()> {
        writer.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    fn write_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
        let bytes = s.as_bytes();
        let len = bytes.len() as u32;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(bytes)?;
        Ok(())
    }

    fn write_properties<W: Write>(
        writer: &mut W,
        props: &HashMap<String, PropertyValue>,
    ) -> Result<()> {
        let count = props.len() as u32;
        writer.write_all(&count.to_le_bytes())?;

        for (key, value) in props {
            Self::write_string(writer, key)?;
            Self::write_property_value(writer, value)?;
        }

        Ok(())
    }

    fn write_property_value<W: Write>(writer: &mut W, value: &PropertyValue) -> Result<()> {
        match value {
            PropertyValue::Null => {
                writer.write_all(&[0u8])?;
            }
            PropertyValue::Bool(b) => {
                writer.write_all(&[1u8])?;
                writer.write_all(&[if *b { 1 } else { 0 }])?;
            }
            PropertyValue::Int(n) => {
                writer.write_all(&[2u8])?;
                writer.write_all(&n.to_le_bytes())?;
            }
            PropertyValue::Float(n) => {
                writer.write_all(&[3u8])?;
                writer.write_all(&n.to_le_bytes())?;
            }
            PropertyValue::String(s) => {
                writer.write_all(&[4u8])?;
                Self::write_string(writer, s)?;
            }
        }
        Ok(())
    }

    // ========== Reader helpers ==========

    fn read_u32<R: Read>(reader: &mut R) -> Result<u32> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn read_u64<R: Read>(reader: &mut R) -> Result<u64> {
        let mut buf = [0u8; 8];
        reader.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_string<R: Read>(reader: &mut R) -> Result<String> {
        let len = Self::read_u32(reader)? as usize;
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        String::from_utf8(buf)
            .map_err(|_| BackupError::CorruptedData("invalid UTF-8 string".to_string()))
    }

    fn read_properties<R: Read>(reader: &mut R) -> Result<HashMap<String, PropertyValue>> {
        let count = Self::read_u32(reader)? as usize;
        let mut props = HashMap::with_capacity(count);

        for _ in 0..count {
            let key = Self::read_string(reader)?;
            let value = Self::read_property_value(reader)?;
            props.insert(key, value);
        }

        Ok(props)
    }

    fn read_property_value<R: Read>(reader: &mut R) -> Result<PropertyValue> {
        let mut type_byte = [0u8; 1];
        reader.read_exact(&mut type_byte)?;

        match type_byte[0] {
            0 => Ok(PropertyValue::Null),
            1 => {
                let mut buf = [0u8; 1];
                reader.read_exact(&mut buf)?;
                Ok(PropertyValue::Bool(buf[0] != 0))
            }
            2 => {
                let mut buf = [0u8; 8];
                reader.read_exact(&mut buf)?;
                Ok(PropertyValue::Int(i64::from_le_bytes(buf)))
            }
            3 => {
                let mut buf = [0u8; 8];
                reader.read_exact(&mut buf)?;
                Ok(PropertyValue::Float(f64::from_le_bytes(buf)))
            }
            4 => {
                let s = Self::read_string(reader)?;
                Ok(PropertyValue::String(s))
            }
            t => Err(BackupError::CorruptedData(format!(
                "unknown property type: {}",
                t
            ))),
        }
    }
}

// ========== Backup Scheduler ==========

/// Schedules periodic backups of a graph.
///
/// Backups are created every `interval_secs` seconds into `output_dir`.
/// File names include the Unix timestamp: `backup_<timestamp>.db.gz`.
/// When the number of backup files exceeds `max_backups`, the oldest files
/// are deleted automatically.
pub struct BackupScheduler {
    interval_secs: u64,
    output_dir: String,
    max_backups: usize,
    callback: Option<BackupCallback>,
}

impl BackupScheduler {
    /// Create a new scheduler.
    ///
    /// # Arguments
    /// * `interval_secs` - How often (in seconds) to create a backup
    /// * `output_dir` - Directory where backup files will be written
    /// * `max_backups` - Maximum number of backup files to keep; older files
    ///   are removed when this limit is exceeded
    pub fn new(interval_secs: u64, output_dir: impl Into<String>, max_backups: usize) -> Self {
        Self {
            interval_secs,
            output_dir: output_dir.into(),
            max_backups,
            callback: None,
        }
    }

    /// Register a callback that is invoked after each successful backup.
    ///
    /// The callback receives a reference to the `BackupMetadata` of the
    /// newly created backup.
    pub fn on_complete(mut self, callback: BackupCallback) -> Self {
        self.callback = Some(callback);
        self
    }

    /// Start the scheduler.  This runs indefinitely (until the task is
    /// cancelled or the process exits).
    ///
    /// # Arguments
    /// * `graph` - Shared reference to the graph protected by an async RwLock
    pub async fn start(self, graph: Arc<RwLock<Graph>>) {
        let interval = tokio::time::Duration::from_secs(self.interval_secs);
        let mut ticker = tokio::time::interval(interval);
        // The first tick fires immediately; skip it so we wait a full interval
        // before the first backup.
        ticker.tick().await;

        loop {
            ticker.tick().await;

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before UNIX epoch")
                .as_secs();

            let path = format!("{}/backup_{}.db.gz", self.output_dir, timestamp);
            let options = BackupOptions::compressed();

            match Backup::create_async(Arc::clone(&graph), &path, &options).await {
                Ok(metadata) => {
                    if let Some(ref cb) = self.callback {
                        cb(&metadata);
                    }
                    // Prune old backups
                    if let Err(e) = Self::prune_old_backups(&self.output_dir, self.max_backups) {
                        eprintln!("BackupScheduler: failed to prune old backups: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("BackupScheduler: backup failed: {}", e);
                }
            }
        }
    }

    /// Remove the oldest backup files from `output_dir` if there are more
    /// than `max_backups` files.
    fn prune_old_backups(output_dir: &str, max_backups: usize) -> std::io::Result<()> {
        let dir = Path::new(output_dir);
        let mut files: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("backup_") && name.ends_with(".db.gz")
            })
            .collect();

        if files.len() <= max_backups {
            return Ok(());
        }

        // Sort by modification time (oldest first)
        files.sort_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });

        let excess = files.len() - max_backups;
        for entry in files.iter().take(excess) {
            std::fs::remove_file(entry.path())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::time::Duration;

    fn tmp_path(name: &str) -> String {
        format!("/tmp/{}", name)
    }

    #[test]
    fn test_backup_restore_empty_graph() {
        let graph = Graph::new();
        let path = tmp_path("test_empty.backup");

        let options = BackupOptions::default();
        let metadata = Backup::create(&graph, &path, &options).unwrap();

        assert_eq!(metadata.node_count, 0);
        assert_eq!(metadata.edge_count, 0);
        assert!(!metadata.compressed);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 0);
        assert_eq!(restored.edge_count(), 0);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_restore_with_data() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(alice) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }
        if let Some(node) = graph.get_node_mut(bob) {
            node.set_property("name", "Bob");
            node.set_property("age", 25);
        }

        let edge_id = graph.create_edge(alice, bob, "KNOWS").unwrap();
        if let Some(edge) = graph.get_edge_mut(edge_id) {
            edge.set_property("since", 2020);
        }

        let path = tmp_path("test_with_data.backup");
        let options = BackupOptions::default();
        let metadata = Backup::create(&graph, &path, &options).unwrap();

        assert_eq!(metadata.node_count, 2);
        assert_eq!(metadata.edge_count, 1);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 2);
        assert_eq!(restored.edge_count(), 1);

        // Verify node data
        let nodes: Vec<_> = restored.nodes().collect();
        assert_eq!(nodes.len(), 2);

        // Verify edges
        let edges: Vec<_> = restored.edges().collect();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].label, "KNOWS");
        assert_eq!(
            edges[0].properties.get("since"),
            Some(&PropertyValue::Int(2020))
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_restore_compressed() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(alice) {
            node.set_property("name", "Alice");
            node.set_property("bio", "A very long biography that should compress well when using gzip compression because it has lots of repetitive text");
        }
        if let Some(node) = graph.get_node_mut(bob) {
            node.set_property("name", "Bob");
            node.set_property("bio", "A very long biography that should compress well when using gzip compression because it has lots of repetitive text");
        }

        graph.create_edge(alice, bob, "KNOWS").unwrap();

        let path = tmp_path("test_compressed.backup");
        let options = BackupOptions::compressed();
        let metadata = Backup::create(&graph, &path, &options).unwrap();

        assert_eq!(metadata.node_count, 2);
        assert_eq!(metadata.edge_count, 1);
        assert!(metadata.compressed);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 2);
        assert_eq!(restored.edge_count(), 1);

        // Verify data integrity
        let nodes: Vec<_> = restored.nodes().collect();
        let alice_node = nodes
            .iter()
            .find(|n| n.properties.get("name") == Some(&PropertyValue::String("Alice".to_string())))
            .unwrap();
        assert!(alice_node.properties.get("bio").is_some());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_metadata() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        graph.create_node("Person");

        let path = tmp_path("test_metadata.backup");
        let options =
            BackupOptions::default().with_description("Test backup for metadata inspection");

        let created_metadata = Backup::create(&graph, &path, &options).unwrap();

        let read_metadata = Backup::metadata(&path).unwrap();

        assert_eq!(read_metadata.version, created_metadata.version);
        assert_eq!(read_metadata.timestamp, created_metadata.timestamp);
        assert_eq!(read_metadata.node_count, 2);
        assert_eq!(read_metadata.edge_count, 0);
        assert_eq!(read_metadata.compressed, false);
        assert_eq!(
            read_metadata.description,
            "Test backup for metadata inspection"
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_verify() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        graph.create_node("Person");

        let path = tmp_path("test_verify.backup");
        let options = BackupOptions::default();
        Backup::create(&graph, &path, &options).unwrap();

        let result = Backup::verify(&path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_verify_corrupted() {
        let path = tmp_path("test_corrupted.backup");

        // Create a file with valid magic but corrupted data
        let mut file = File::create(&path).unwrap();
        file.write_all(MAGIC).unwrap();
        file.write_all(&VERSION.to_le_bytes()).unwrap();
        file.write_all(&0u64.to_le_bytes()).unwrap(); // timestamp
        file.write_all(&5u64.to_le_bytes()).unwrap(); // node_count (wrong)
        file.write_all(&0u64.to_le_bytes()).unwrap(); // edge_count
        file.write_all(&[0u8]).unwrap(); // compressed
        file.write_all(&0u32.to_le_bytes()).unwrap(); // description length
        // Missing actual graph data
        drop(file);

        let result = Backup::verify(&path);
        assert!(result.is_err());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_restore_roundtrip_properties() {
        let mut graph = Graph::new();
        let id = graph.create_node("Test");

        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("null_val", PropertyValue::Null);
            node.set_property("bool_val", true);
            node.set_property("int_val", 42i64);
            node.set_property("float_val", 3.14159f64);
            node.set_property("string_val", "hello world");
        }

        let path = tmp_path("test_properties.backup");
        let options = BackupOptions::default();
        Backup::create(&graph, &path, &options).unwrap();

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 1);

        let node = restored.nodes().next().unwrap();
        assert_eq!(node.label, "Test");
        assert_eq!(node.properties.get("null_val"), Some(&PropertyValue::Null));
        assert_eq!(
            node.properties.get("bool_val"),
            Some(&PropertyValue::Bool(true))
        );
        assert_eq!(
            node.properties.get("int_val"),
            Some(&PropertyValue::Int(42))
        );
        assert_eq!(
            node.properties.get("float_val"),
            Some(&PropertyValue::Float(3.14159))
        );
        assert_eq!(
            node.properties.get("string_val"),
            Some(&PropertyValue::String("hello world".to_string()))
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_with_description() {
        let graph = Graph::new();
        let path = tmp_path("test_description.backup");

        let options =
            BackupOptions::compressed().with_description("Production backup - Friday 5pm");

        let metadata = Backup::create(&graph, &path, &options).unwrap();
        assert_eq!(metadata.description, "Production backup - Friday 5pm");
        assert!(metadata.compressed);

        let read_metadata = Backup::metadata(&path).unwrap();
        assert_eq!(read_metadata.description, "Production backup - Friday 5pm");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_invalid_magic() {
        let path = tmp_path("test_invalid_magic.backup");
        std::fs::write(&path, b"INVALID!").unwrap();

        let result = Backup::restore(&path);
        assert!(matches!(result, Err(BackupError::InvalidMagic)));

        let metadata_result = Backup::metadata(&path);
        assert!(matches!(metadata_result, Err(BackupError::InvalidMagic)));

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_backup_unsupported_version() {
        let path = tmp_path("test_unsupported_version.backup");

        let mut file = File::create(&path).unwrap();
        file.write_all(MAGIC).unwrap();
        file.write_all(&999u32.to_le_bytes()).unwrap(); // unsupported version
        drop(file);

        let result = Backup::restore(&path);
        assert!(matches!(result, Err(BackupError::UnsupportedVersion(999))));

        std::fs::remove_file(path).ok();
    }

    // ========== Async backup tests ==========

    #[tokio::test]
    async fn test_create_async_basic() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(a) {
            node.set_property("name", "Alice");
        }

        let shared = Arc::new(RwLock::new(graph));
        let path = tmp_path("test_async_basic.backup");
        let options = BackupOptions::default();

        let metadata = Backup::create_async(Arc::clone(&shared), &path, &options)
            .await
            .unwrap();

        assert_eq!(metadata.node_count, 1);
        assert!(!metadata.compressed);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 1);

        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn test_create_async_compressed() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        graph.create_node("Company");

        let shared = Arc::new(RwLock::new(graph));
        let path = tmp_path("test_async_compressed.backup");
        let options = BackupOptions::compressed();

        let metadata = Backup::create_async(Arc::clone(&shared), &path, &options)
            .await
            .unwrap();

        assert_eq!(metadata.node_count, 2);
        assert!(metadata.compressed);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 2);

        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn test_create_async_concurrent_reads() {
        // Verify that the read lock allows concurrent readers
        let mut graph = Graph::new();
        graph.create_node("Node");
        let shared = Arc::new(RwLock::new(graph));

        let path1 = tmp_path("test_async_concurrent1.backup");
        let path2 = tmp_path("test_async_concurrent2.backup");

        let options = BackupOptions::default();
        let (r1, r2) = tokio::join!(
            Backup::create_async(Arc::clone(&shared), &path1, &options),
            Backup::create_async(Arc::clone(&shared), &path2, &options),
        );

        assert!(r1.is_ok());
        assert!(r2.is_ok());

        std::fs::remove_file(path1).ok();
        std::fs::remove_file(path2).ok();
    }

    // ========== Scheduler tests ==========

    #[tokio::test]
    async fn test_scheduler_creates_backups() {
        let output_dir = tmp_path("scheduler_test_create");
        std::fs::create_dir_all(&output_dir).unwrap();

        let graph = Arc::new(RwLock::new(Graph::new()));
        let dir_clone = output_dir.clone();

        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let scheduler = BackupScheduler::new(1, output_dir.clone(), 10).on_complete(Box::new(
            move |_meta| {
                counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        ));

        // Run scheduler for just over 2 seconds; expect at least 2 backups
        let handle = tokio::spawn(scheduler.start(Arc::clone(&graph)));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        handle.abort();

        let files: Vec<_> = std::fs::read_dir(&dir_clone)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("backup_")
            })
            .collect();

        assert!(
            files.len() >= 2,
            "expected at least 2 backup files, got {}",
            files.len()
        );

        std::fs::remove_dir_all(&dir_clone).ok();
    }

    #[tokio::test]
    async fn test_scheduler_max_backups_pruning() {
        let output_dir = tmp_path("scheduler_test_prune");
        std::fs::create_dir_all(&output_dir).unwrap();

        let graph = Arc::new(RwLock::new(Graph::new()));
        let max_backups = 2usize;

        let scheduler = BackupScheduler::new(1, output_dir.clone(), max_backups);

        // Run for ~4 seconds; should create ~4 backups but keep only max_backups
        let handle = tokio::spawn(scheduler.start(Arc::clone(&graph)));
        tokio::time::sleep(Duration::from_millis(4500)).await;
        handle.abort();

        let files: Vec<_> = std::fs::read_dir(&output_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("backup_")
            })
            .collect();

        assert!(
            files.len() <= max_backups,
            "expected at most {} backup files, got {}",
            max_backups,
            files.len()
        );

        std::fs::remove_dir_all(&output_dir).ok();
    }

    #[tokio::test]
    async fn test_scheduler_callback_invoked() {
        let output_dir = tmp_path("scheduler_test_callback");
        std::fs::create_dir_all(&output_dir).unwrap();

        let graph = Arc::new(RwLock::new(Graph::new()));

        // Use Arc<Mutex<Vec<...>>> to collect callback results
        let results: Arc<Mutex<Vec<BackupMetadata>>> = Arc::new(Mutex::new(Vec::new()));
        let results_clone = Arc::clone(&results);

        let scheduler = BackupScheduler::new(1, output_dir.clone(), 10).on_complete(Box::new(
            move |meta| {
                results_clone.lock().unwrap().push(meta.clone());
            },
        ));

        let handle = tokio::spawn(scheduler.start(Arc::clone(&graph)));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        handle.abort();

        let collected = results.lock().unwrap();
        assert!(
            collected.len() >= 2,
            "expected callback to be called at least 2 times, got {}",
            collected.len()
        );
        // All metadata should have version == VERSION
        for meta in collected.iter() {
            assert_eq!(meta.version, VERSION);
            assert!(meta.compressed); // scheduler uses compressed() options
        }

        std::fs::remove_dir_all(&output_dir).ok();
    }

    #[test]
    fn test_prune_old_backups_removes_excess() {
        let dir = tmp_path("prune_test_dir");
        std::fs::create_dir_all(&dir).unwrap();

        // Create 5 fake backup files
        for i in 0..5 {
            let path = format!("{}/backup_{}.db.gz", dir, i);
            std::fs::write(&path, b"fake").unwrap();
            // Sleep briefly so mtime differs
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        BackupScheduler::prune_old_backups(&dir, 3).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        assert_eq!(remaining.len(), 3, "expected exactly 3 files to remain");

        std::fs::remove_dir_all(&dir).ok();
    }
}
