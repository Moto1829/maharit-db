use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use maharit_core::{Graph, IndexDefinition, PropertyIndex, PropertyValue};
use thiserror::Error;
use tokio::sync::RwLock;

/// Backup file format magic number
const MAGIC: &[u8; 8] = b"MHRTBKUP";
/// Backup format version
const VERSION: u32 = 1;

/// Compression algorithm for backup files.
///
/// Stored as a 1-byte tag in the file header for backward compatibility:
/// - `0` = None (uncompressed)
/// - `1` = Gzip (previously stored as `compressed = true`)
/// - `2` = Zstd
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionType {
    #[default]
    None,
    Gzip,
    Zstd,
}

impl CompressionType {
    fn to_byte(self) -> u8 {
        match self {
            CompressionType::None => 0,
            CompressionType::Gzip => 1,
            CompressionType::Zstd => 2,
        }
    }

    fn from_byte(b: u8) -> Self {
        match b {
            1 => CompressionType::Gzip,
            2 => CompressionType::Zstd,
            _ => CompressionType::None,
        }
    }
}

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

    #[error("WAL error: {0}")]
    Wal(String),
}

pub type Result<T> = std::result::Result<T, BackupError>;

/// Options for creating a backup
#[derive(Debug, Clone, Default)]
pub struct BackupOptions {
    /// Compression algorithm to use (default: None)
    pub compression: CompressionType,
    /// Optional description of the backup
    pub description: String,
}

impl BackupOptions {
    /// Create backup options with gzip compression enabled.
    pub fn compressed() -> Self {
        Self {
            compression: CompressionType::Gzip,
            description: String::new(),
        }
    }

    /// Create backup options with zstd compression enabled.
    pub fn compressed_zstd() -> Self {
        Self {
            compression: CompressionType::Zstd,
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
    /// Compression algorithm used for this backup
    pub compression: CompressionType,
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
            compression: options.compression,
            description: options.description.clone(),
        };

        // Write header
        writer.write_all(MAGIC)?;
        writer.write_all(&metadata.version.to_le_bytes())?;
        writer.write_all(&metadata.timestamp.to_le_bytes())?;
        writer.write_all(&metadata.node_count.to_le_bytes())?;
        writer.write_all(&metadata.edge_count.to_le_bytes())?;
        writer.write_all(&[metadata.compression.to_byte()])?;
        Self::write_string(&mut writer, &metadata.description)?;

        // Serialize graph data to a buffer
        let graph_data = Self::serialize_graph(graph, &[])?;

        // Write graph data (compress if requested)
        Self::write_compressed(&mut writer, &graph_data, options.compression)?;

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
            let data = Self::serialize_graph(&g, &[])?;
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
            compression: options.compression,
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
        writer.write_all(&[metadata.compression.to_byte()])?;
        Self::write_string(&mut writer, &metadata.description)?;

        Self::write_compressed(&mut writer, &graph_data, options.compression)?;

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

        let mut compression_byte = [0u8; 1];
        reader.read_exact(&mut compression_byte)?;
        let compression = CompressionType::from_byte(compression_byte[0]);

        let _description = Self::read_string(&mut reader)?;

        // Read graph data (decompress if needed)
        let graph_data = Self::read_compressed(reader, compression)?;

        // Deserialize graph
        Self::deserialize_graph(&graph_data)
    }

    /// Create a backup of a graph along with its property index definitions.
    ///
    /// The index definitions are appended to the serialized graph data so that
    /// [`restore_with_index`] can reconstruct a fully populated `PropertyIndex`.
    /// The actual indexed data (which nodes map to which values) is rebuilt
    /// during restore by iterating the nodes in the restored graph.
    ///
    /// # Arguments
    /// * `graph`          - The graph to back up
    /// * `property_index` - The property index whose definitions should be saved
    /// * `path`           - Destination file path
    /// * `options`        - Compression / description options
    ///
    /// # Returns
    /// The metadata of the created backup
    pub fn create_with_index(
        graph: &Graph,
        property_index: &PropertyIndex,
        path: impl AsRef<Path>,
        options: &BackupOptions,
    ) -> Result<BackupMetadata> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_secs();

        let metadata = BackupMetadata {
            version: VERSION,
            timestamp,
            node_count: graph.node_count() as u64,
            edge_count: graph.edge_count() as u64,
            compression: options.compression,
            description: options.description.clone(),
        };

        // Write header
        writer.write_all(MAGIC)?;
        writer.write_all(&metadata.version.to_le_bytes())?;
        writer.write_all(&metadata.timestamp.to_le_bytes())?;
        writer.write_all(&metadata.node_count.to_le_bytes())?;
        writer.write_all(&metadata.edge_count.to_le_bytes())?;
        writer.write_all(&[metadata.compression.to_byte()])?;
        Self::write_string(&mut writer, &metadata.description)?;

        // Collect index definitions and serialize graph + indexes together
        let index_defs: Vec<IndexDefinition> = property_index
            .list_indexes()
            .into_iter()
            .cloned()
            .collect();
        let graph_data = Self::serialize_graph(graph, &index_defs)?;

        Self::write_compressed(&mut writer, &graph_data, options.compression)?;

        writer.flush()?;
        Ok(metadata)
    }

    /// Restore a graph and its property index from a backup file.
    ///
    /// The index definitions embedded in the backup are used to recreate the
    /// `PropertyIndex`.  Each definition is registered via
    /// `PropertyIndex::create_index` and then all matching node properties are
    /// re-indexed by iterating the restored graph.
    ///
    /// Backups created with [`create`] (which contain no index section) are
    /// handled gracefully: an empty `PropertyIndex` is returned.
    ///
    /// # Arguments
    /// * `path` - Path to the backup file
    ///
    /// # Returns
    /// A tuple of `(Graph, PropertyIndex)`
    pub fn restore_with_index(path: impl AsRef<Path>) -> Result<(Graph, PropertyIndex)> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Verify magic
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BackupError::InvalidMagic);
        }

        // Read and validate header
        let version = Self::read_u32(&mut reader)?;
        if version != VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }

        let _timestamp = Self::read_u64(&mut reader)?;
        let _node_count = Self::read_u64(&mut reader)?;
        let _edge_count = Self::read_u64(&mut reader)?;

        let mut compression_byte = [0u8; 1];
        reader.read_exact(&mut compression_byte)?;
        let compression = CompressionType::from_byte(compression_byte[0]);

        let _description = Self::read_string(&mut reader)?;

        // Decompress graph section
        let graph_data = Self::read_compressed(reader, compression)?;

        // Deserialize graph and index definitions
        let (graph, index_defs) = Self::deserialize_graph_internal(&graph_data)?;

        // Rebuild PropertyIndex: register definitions, then re-index node data
        let mut property_index = PropertyIndex::new();
        for def in &index_defs {
            property_index.create_index(def.clone());
        }

        for def in &index_defs {
            for node in graph.nodes() {
                if node.labels.contains(&def.label) {
                    if let Some(val) = node.get_property(&def.property) {
                        property_index.index_property(node.id, &def.property, val);
                    }
                }
            }
        }

        Ok((graph, property_index))
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

        let mut compression_byte = [0u8; 1];
        reader.read_exact(&mut compression_byte)?;
        let compression = CompressionType::from_byte(compression_byte[0]);

        let description = Self::read_string(&mut reader)?;

        Ok(BackupMetadata {
            version,
            timestamp,
            node_count,
            edge_count,
            compression,
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

    fn serialize_graph(graph: &Graph, indexes: &[IndexDefinition]) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();

        // Write node count and edge count
        let node_count = graph.node_count() as u64;
        let edge_count = graph.edge_count() as u64;
        buffer.write_all(&node_count.to_le_bytes())?;
        buffer.write_all(&edge_count.to_le_bytes())?;

        // Write nodes
        for node in graph.nodes() {
            Self::write_u64(&mut buffer, node.id)?;
            // Store labels as colon-separated for backward compat
            Self::write_string(&mut buffer, &node.labels.join(":"))?;
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

        // Write index definitions
        let index_count = indexes.len() as u32;
        buffer.write_all(&index_count.to_le_bytes())?;
        for def in indexes {
            Self::write_string(&mut buffer, &def.label)?;
            Self::write_string(&mut buffer, &def.property)?;
            buffer.write_all(&[if def.unique { 1u8 } else { 0u8 }])?;
        }

        Ok(buffer)
    }

    fn deserialize_graph(data: &[u8]) -> Result<Graph> {
        Ok(Self::deserialize_graph_internal(data)?.0)
    }

    fn deserialize_graph_internal(data: &[u8]) -> Result<(Graph, Vec<IndexDefinition>)> {
        let mut reader = std::io::Cursor::new(data);

        // Read node count and edge count
        let node_count = Self::read_u64(&mut reader)?;
        let edge_count = Self::read_u64(&mut reader)?;

        let mut graph = Graph::new();
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        // Read nodes
        for _ in 0..node_count {
            let old_id = Self::read_u64(&mut reader)?;
            let labels_str = Self::read_string(&mut reader)?;
            let labels: Vec<String> = if labels_str.is_empty() {
                vec![]
            } else {
                labels_str.split(':').map(|s| s.to_string()).collect()
            };
            let properties = Self::read_properties(&mut reader)?;

            let new_id = graph.create_node_with_labels(labels);
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

        // Try to read index definitions (may be absent in old format)
        let indexes = match Self::read_u32_or_eof(&mut reader) {
            Ok(index_count) => {
                let mut defs = Vec::with_capacity(index_count as usize);
                for _ in 0..index_count {
                    let label = Self::read_string(&mut reader)?;
                    let property = Self::read_string(&mut reader)?;
                    let mut unique_buf = [0u8; 1];
                    reader.read_exact(&mut unique_buf)?;
                    let unique = unique_buf[0] != 0;
                    let mut def = IndexDefinition::new(label, property);
                    def.unique = unique;
                    defs.push(def);
                }
                defs
            }
            Err(BackupError::Io(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                vec![]
            }
            Err(e) => return Err(e),
        };

        Ok((graph, indexes))
    }

    // ========== Writer helpers ==========

    fn write_u64<W: Write>(writer: &mut W, value: u64) -> Result<()> {
        writer.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    /// Compress `data` with the chosen algorithm and write to `writer`.
    fn write_compressed<W: Write>(writer: &mut W, data: &[u8], compression: CompressionType) -> Result<()> {
        match compression {
            CompressionType::None => {
                writer.write_all(data)?;
            }
            CompressionType::Gzip => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data)?;
                let compressed = encoder.finish()?;
                writer.write_all(&compressed)?;
            }
            CompressionType::Zstd => {
                let compressed = zstd::encode_all(data, 3)
                    .map_err(std::io::Error::other)?;
                writer.write_all(&compressed)?;
            }
        }
        Ok(())
    }

    /// Read and decompress data from `reader` based on `compression`.
    fn read_compressed<R: Read>(reader: R, compression: CompressionType) -> Result<Vec<u8>> {
        match compression {
            CompressionType::None => {
                let mut data = Vec::new();
                let mut r = reader;
                r.read_to_end(&mut data)?;
                Ok(data)
            }
            CompressionType::Gzip => {
                let mut decoder = GzDecoder::new(reader);
                let mut decompressed = Vec::new();
                decoder.read_to_end(&mut decompressed)?;
                Ok(decompressed)
            }
            CompressionType::Zstd => {
                let mut r = reader;
                let mut compressed = Vec::new();
                r.read_to_end(&mut compressed)?;
                let decompressed = zstd::decode_all(compressed.as_slice())
                    .map_err(std::io::Error::other)?;
                Ok(decompressed)
            }
        }
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
            PropertyValue::Date(d) => {
                writer.write_all(&[5u8])?;
                writer.write_all(&d.to_le_bytes())?;
            }
            PropertyValue::DateTime(ms) => {
                writer.write_all(&[6u8])?;
                writer.write_all(&ms.to_le_bytes())?;
            }
            PropertyValue::Duration { months, days, millis } => {
                writer.write_all(&[7u8])?;
                writer.write_all(&months.to_le_bytes())?;
                writer.write_all(&days.to_le_bytes())?;
                writer.write_all(&millis.to_le_bytes())?;
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

    /// Like `read_u32` but returns `Err(BackupError::Io(UnexpectedEof))` if
    /// the reader is already at EOF (no bytes available), allowing callers to
    /// distinguish "no more data" from a mid-field truncation.
    fn read_u32_or_eof<R: Read>(reader: &mut R) -> Result<u32> {
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
            5 => {
                let mut buf = [0u8; 4];
                reader.read_exact(&mut buf)?;
                Ok(PropertyValue::Date(i32::from_le_bytes(buf)))
            }
            6 => {
                let mut buf = [0u8; 8];
                reader.read_exact(&mut buf)?;
                Ok(PropertyValue::DateTime(i64::from_le_bytes(buf)))
            }
            7 => {
                let mut buf4 = [0u8; 4];
                let mut buf8 = [0u8; 8];
                reader.read_exact(&mut buf4)?;
                let months = i32::from_le_bytes(buf4);
                reader.read_exact(&mut buf4)?;
                let days = i32::from_le_bytes(buf4);
                reader.read_exact(&mut buf8)?;
                let millis = i64::from_le_bytes(buf8);
                Ok(PropertyValue::Duration { months, days, millis })
            }
            t => Err(BackupError::CorruptedData(format!(
                "unknown property type: {}",
                t
            ))),
        }
    }
}

// ========== Incremental backup ==========

/// Magic bytes for incremental backup files.
const INCR_MAGIC: &[u8; 8] = b"MHRTINCR";
/// Incremental backup format version.
const INCR_VERSION: u32 = 1;

/// Metadata describing an incremental backup.
///
/// An incremental backup records only the nodes and edges that were modified
/// (or created) after `base_timestamp` and the IDs of anything that was
/// deleted since that point.
#[derive(Debug, Clone)]
pub struct IncrementalBackupMetadata {
    /// ID of the base full backup this increment builds on.
    pub base_backup_id: String,
    /// Unix timestamp (seconds) of the base backup.
    pub base_timestamp: u64,
    /// Unique ID for this incremental backup.
    pub incremental_id: String,
    /// Unix timestamp (seconds) when this incremental backup was created.
    pub timestamp: u64,
    /// Node IDs that were added or modified since the base backup.
    pub changed_node_ids: Vec<u64>,
    /// Edge IDs that were added or modified since the base backup.
    pub changed_edge_ids: Vec<u64>,
    /// Node IDs that were deleted since the base backup.
    pub deleted_node_ids: Vec<u64>,
    /// Edge IDs that were deleted since the base backup.
    pub deleted_edge_ids: Vec<u64>,
}

impl Backup {
    // ------------------------------------------------------------------
    // Incremental backup
    // ------------------------------------------------------------------

    /// Create an incremental backup containing only the nodes and edges that
    /// were recorded as changed in `wal_path` after `base_timestamp`.
    ///
    /// The WAL is scanned to discover which node/edge IDs were created,
    /// modified, or deleted after the given timestamp.  For nodes/edges that
    /// still exist in `graph` their full data is serialised into the output
    /// file.  Deleted IDs are stored in the metadata so that
    /// [`restore_incremental`] can remove them when applying the diff.
    ///
    /// # File format
    /// `INCR_MAGIC | INCR_VERSION | base_backup_id_str | base_timestamp_u64 |
    ///  incremental_id_str | timestamp_u64 |
    ///  changed_node_count_u64 | [node_data]* |
    ///  changed_edge_count_u64 | [edge_data]* |
    ///  deleted_node_count_u64 | [node_id_u64]* |
    ///  deleted_edge_count_u64 | [edge_id_u64]*`
    pub fn create_incremental(
        graph: &Graph,
        base_backup_id: &str,
        base_timestamp: u64,
        wal_path: &str,
        output_path: &str,
        options: &BackupOptions,
    ) -> Result<IncrementalBackupMetadata> {
        use crate::wal::{RecordPayload, Wal};

        // Scan WAL to find what changed after base_timestamp.
        let wal = Wal::open(wal_path).map_err(|e| BackupError::Wal(e.to_string()))?;
        let (all_records, _) = wal.read_all_for_incremental(base_timestamp);

        let mut changed_nodes: HashSet<u64> = HashSet::new();
        let mut changed_edges: HashSet<u64> = HashSet::new();
        let mut deleted_nodes: HashSet<u64> = HashSet::new();
        let mut deleted_edges: HashSet<u64> = HashSet::new();

        for record in &all_records {
            match &record.payload {
                RecordPayload::CreateNode { node_id, .. }
                | RecordPayload::SetNodeProperty { node_id, .. } => {
                    changed_nodes.insert(*node_id);
                }
                RecordPayload::DeleteNode { node_id } => {
                    changed_nodes.remove(node_id);
                    deleted_nodes.insert(*node_id);
                }
                RecordPayload::CreateEdge { edge_id, .. }
                | RecordPayload::SetEdgeProperty { edge_id, .. } => {
                    changed_edges.insert(*edge_id);
                }
                RecordPayload::DeleteEdge { edge_id } => {
                    changed_edges.remove(edge_id);
                    deleted_edges.insert(*edge_id);
                }
                RecordPayload::Checkpoint { .. } => {}
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_secs();

        let incremental_id = format!("incr-{}", timestamp);

        // Collect changed node IDs that still exist in the graph.
        let changed_node_ids: Vec<u64> = changed_nodes
            .iter()
            .filter(|&&id| graph.get_node(id).is_some())
            .copied()
            .collect();
        let changed_edge_ids: Vec<u64> = changed_edges
            .iter()
            .filter(|&&id| graph.get_edge(id).is_some())
            .copied()
            .collect();
        let deleted_node_ids: Vec<u64> = deleted_nodes.into_iter().collect();
        let deleted_edge_ids: Vec<u64> = deleted_edges.into_iter().collect();

        // Serialise to file.
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);

        // Header
        writer.write_all(INCR_MAGIC)?;
        writer.write_all(&INCR_VERSION.to_le_bytes())?;
        Self::write_string(&mut writer, base_backup_id)?;
        writer.write_all(&base_timestamp.to_le_bytes())?;
        Self::write_string(&mut writer, &incremental_id)?;
        writer.write_all(&timestamp.to_le_bytes())?;

        // Compress flag
        writer.write_all(&[options.compression.to_byte()])?;
        Self::write_string(&mut writer, &options.description)?;

        // Build the payload into a buffer (so it can be optionally compressed)
        let mut payload: Vec<u8> = Vec::new();

        // Changed nodes
        Self::write_u64(&mut payload, changed_node_ids.len() as u64)?;
        for &node_id in &changed_node_ids {
            if let Some(node) = graph.get_node(node_id) {
                Self::write_u64(&mut payload, node.id)?;
                Self::write_string(&mut payload, &node.labels.join(":"))?;
                Self::write_properties(&mut payload, &node.properties)?;
            }
        }

        // Changed edges
        Self::write_u64(&mut payload, changed_edge_ids.len() as u64)?;
        for &edge_id in &changed_edge_ids {
            if let Some(edge) = graph.get_edge(edge_id) {
                Self::write_u64(&mut payload, edge.id)?;
                Self::write_u64(&mut payload, edge.from)?;
                Self::write_u64(&mut payload, edge.to)?;
                Self::write_string(&mut payload, &edge.label)?;
                Self::write_properties(&mut payload, &edge.properties)?;
            }
        }

        // Deleted node IDs
        Self::write_u64(&mut payload, deleted_node_ids.len() as u64)?;
        for &id in &deleted_node_ids {
            Self::write_u64(&mut payload, id)?;
        }

        // Deleted edge IDs
        Self::write_u64(&mut payload, deleted_edge_ids.len() as u64)?;
        for &id in &deleted_edge_ids {
            Self::write_u64(&mut payload, id)?;
        }

        // Write payload (compressed or raw)
        Self::write_compressed(&mut writer, &payload, options.compression)?;

        writer.flush()?;

        Ok(IncrementalBackupMetadata {
            base_backup_id: base_backup_id.to_string(),
            base_timestamp,
            incremental_id,
            timestamp,
            changed_node_ids,
            changed_edge_ids,
            deleted_node_ids,
            deleted_edge_ids,
        })
    }

    /// Restore a graph by first loading `base_path` (full backup) and then
    /// applying the changes recorded in `incremental_path`.
    pub fn restore_incremental(
        base_path: &str,
        incremental_path: &str,
    ) -> Result<Graph> {
        // Load base backup
        let mut graph = Self::restore(base_path)?;

        // Parse incremental file
        let file = File::open(incremental_path)?;
        let mut reader = BufReader::new(file);

        // Magic
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != INCR_MAGIC {
            return Err(BackupError::CorruptedData(
                "invalid incremental backup magic".to_string(),
            ));
        }

        // Version
        let version = Self::read_u32(&mut reader)?;
        if version != INCR_VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }

        // Metadata fields
        let _base_backup_id = Self::read_string(&mut reader)?;
        let _base_timestamp = Self::read_u64(&mut reader)?;
        let _incremental_id = Self::read_string(&mut reader)?;
        let _timestamp = Self::read_u64(&mut reader)?;

        // Compressed flag + description
        let mut flag = [0u8; 1];
        reader.read_exact(&mut flag)?;
        let compressed = flag[0] != 0;
        let _description = Self::read_string(&mut reader)?;

        // Decompress payload if needed
        let payload_data: Vec<u8> = if compressed {
            let mut decoder = GzDecoder::new(reader);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out)?;
            out
        } else {
            let mut out = Vec::new();
            reader.read_to_end(&mut out)?;
            out
        };

        let mut cursor = std::io::Cursor::new(payload_data.as_slice());

        // Apply changed nodes
        let node_count = Self::read_u64(&mut cursor)?;
        for _ in 0..node_count {
            let old_id = Self::read_u64(&mut cursor)?;
            let labels_str = Self::read_string(&mut cursor)?;
            let labels: Vec<String> = if labels_str.is_empty() {
                vec![]
            } else {
                labels_str.split(':').map(|s| s.to_string()).collect()
            };
            let properties = Self::read_properties(&mut cursor)?;

            // If the node already exists in the base graph, update it;
            // otherwise create it (the WAL id might already be present if
            // the base backup was taken from the same graph).
            if graph.get_node(old_id).is_none() {
                let new_id = graph.create_node_with_id_and_labels(old_id, labels);
                if let Some(node) = graph.get_node_mut(new_id) {
                    for (k, v) in properties {
                        node.set_property(k, v);
                    }
                }
            } else if let Some(node) = graph.get_node_mut(old_id) {
                node.labels = labels;
                node.properties = std::sync::Arc::new(properties);
            }
        }

        // Apply changed edges
        let edge_count = Self::read_u64(&mut cursor)?;
        for _ in 0..edge_count {
            let old_id = Self::read_u64(&mut cursor)?;
            let from = Self::read_u64(&mut cursor)?;
            let to = Self::read_u64(&mut cursor)?;
            let label = Self::read_string(&mut cursor)?;
            let properties = Self::read_properties(&mut cursor)?;

            if graph.get_edge(old_id).is_none()
                && graph.get_node(from).is_some()
                && graph.get_node(to).is_some()
            {
                let new_eid = graph.create_edge(from, to, &label)?;
                if let Some(edge) = graph.get_edge_mut(new_eid) {
                    for (k, v) in properties {
                        edge.set_property(k, v);
                    }
                }
            } else if let Some(edge) = graph.get_edge_mut(old_id) {
                edge.label = label;
                edge.properties = std::sync::Arc::new(properties);
            }
        }

        // Apply deletions – nodes
        let del_node_count = Self::read_u64(&mut cursor)?;
        for _ in 0..del_node_count {
            let node_id = Self::read_u64(&mut cursor)?;
            graph.delete_node(node_id);
        }

        // Apply deletions – edges
        let del_edge_count = Self::read_u64(&mut cursor)?;
        for _ in 0..del_edge_count {
            let edge_id = Self::read_u64(&mut cursor)?;
            graph.delete_edge(edge_id);
        }

        Ok(graph)
    }

    // ------------------------------------------------------------------
    // Point-in-time recovery (PITR)
    // ------------------------------------------------------------------

    /// Restore a graph from a full backup and replay WAL entries up to
    /// (and including) `target_timestamp`.
    ///
    /// WAL entries with `timestamp <= target_timestamp` are applied in LSN
    /// order.  Entries after the target are ignored.  If `target_timestamp`
    /// exceeds the timestamp of the last WAL entry all entries are applied
    /// (effectively restoring to the latest available state).
    pub fn restore_to_point_in_time(
        base_backup_path: &str,
        wal_path: &str,
        target_timestamp: u64,
    ) -> Result<Graph> {
        use crate::wal::{RecordPayload, Wal};

        // Step 1: restore from full backup.
        let mut graph = Self::restore(base_backup_path)?;

        // Step 2: replay WAL entries up to target_timestamp.
        let wal = Wal::open(wal_path).map_err(|e| BackupError::Wal(e.to_string()))?;
        let (records, _) = wal.read_all_for_incremental(0); // 0 = all records

        // id_map: maps WAL node_id → graph node_id (since graph.create_node
        // may assign different IDs on restore).
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        // Pre-populate id_map from nodes that are already in the restored
        // graph so that property updates and edge creations can find them.
        for node in graph.nodes() {
            id_map.insert(node.id, node.id);
        }

        for record in records {
            if record.timestamp > target_timestamp {
                // All subsequent records have higher or equal LSN timestamps
                // only when the WAL was written in order; we still continue
                // iterating to be safe, but skip entries past the cutoff.
                continue;
            }

            match &record.payload {
                RecordPayload::CreateNode { node_id, label } => {
                    let new_id = graph.create_node(label.as_str());
                    id_map.insert(*node_id, new_id);
                }
                RecordPayload::DeleteNode { node_id } => {
                    let actual = id_map.get(node_id).copied().unwrap_or(*node_id);
                    graph.delete_node(actual);
                    id_map.remove(node_id);
                }
                RecordPayload::CreateEdge {
                    edge_id: _,
                    from,
                    to,
                    label,
                } => {
                    let actual_from = id_map.get(from).copied().unwrap_or(*from);
                    let actual_to = id_map.get(to).copied().unwrap_or(*to);
                    if graph.get_node(actual_from).is_some()
                        && graph.get_node(actual_to).is_some()
                    {
                        let _ = graph.create_edge(actual_from, actual_to, label.as_str());
                    }
                }
                RecordPayload::DeleteEdge { edge_id } => {
                    graph.delete_edge(*edge_id);
                }
                RecordPayload::SetNodeProperty {
                    node_id,
                    key,
                    value,
                } => {
                    let actual = id_map.get(node_id).copied().unwrap_or(*node_id);
                    if let Some(node) = graph.get_node_mut(actual) {
                        node.set_property(key.clone(), value.clone());
                    }
                }
                RecordPayload::SetEdgeProperty {
                    edge_id,
                    key,
                    value,
                } => {
                    if let Some(edge) = graph.get_edge_mut(*edge_id) {
                        edge.set_property(key.clone(), value.clone());
                    }
                }
                RecordPayload::Checkpoint { .. } => {}
            }
        }

        Ok(graph)
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
        assert_eq!(metadata.compression, CompressionType::None);

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
        assert_eq!(metadata.compression, CompressionType::Gzip);

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
    fn test_backup_restore_zstd() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(alice) {
            node.set_property("name", "Alice");
            node.set_property("bio", "A very long biography that should compress well when using zstd compression because it has lots of repetitive text");
        }
        if let Some(node) = graph.get_node_mut(bob) {
            node.set_property("name", "Bob");
            node.set_property("bio", "A very long biography that should compress well when using zstd compression because it has lots of repetitive text");
        }

        graph.create_edge(alice, bob, "KNOWS").unwrap();

        let path = tmp_path("test_zstd_compressed.backup");
        let options = BackupOptions::compressed_zstd();
        let metadata = Backup::create(&graph, &path, &options).unwrap();

        assert_eq!(metadata.node_count, 2);
        assert_eq!(metadata.edge_count, 1);
        assert_eq!(metadata.compression, CompressionType::Zstd);

        let restored = Backup::restore(&path).unwrap();
        assert_eq!(restored.node_count(), 2);
        assert_eq!(restored.edge_count(), 1);

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
        assert_eq!(read_metadata.compression, CompressionType::None);
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
        assert!(node.has_label("Test"));
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
        assert_eq!(metadata.compression, CompressionType::Gzip);

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
        assert_eq!(metadata.compression, CompressionType::None);

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
        assert_eq!(metadata.compression, CompressionType::Gzip);

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
            assert_ne!(meta.compression, CompressionType::None); // scheduler uses compressed() options
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

    // ========== Incremental backup and PITR tests ==========

    #[test]
    fn test_incremental_backup_contains_only_changed_nodes() {
        use crate::wal::{RecordPayload, RecordType, Wal};

        let base_ts: u64 = 1_000_000;
        let wal_path = tmp_path("incr_test_basic.wal");
        let base_backup_path = tmp_path("incr_test_base.backup");
        let incr_path = tmp_path("incr_test_delta.ibackup");

        // Build a graph with node 0 (pre-existing) and node ids 10 and 11
        // which come from the WAL.
        let mut graph = Graph::new();
        let old_node = graph.create_node("OldNode"); // id = 0, existed before base_ts
        graph.create_node_with_id(10, "Thing");
        if let Some(n) = graph.get_node_mut(10) {
            n.set_property("value", 42i64);
        }
        graph.create_node_with_id(11, "Other");

        // Create a base full backup.
        Backup::create(&graph, &base_backup_path, &BackupOptions::default()).unwrap();

        // Write a WAL with changes that happened AFTER base_ts.
        let mut wal = Wal::open(&wal_path).unwrap();
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 10,
                label: "Thing".to_string(),
            },
            base_ts + 1,
        )
        .unwrap();
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 11,
                label: "Other".to_string(),
            },
            base_ts + 2,
        )
        .unwrap();
        wal.sync().unwrap();

        let meta = Backup::create_incremental(
            &graph,
            "base-001",
            base_ts,
            &wal_path,
            &incr_path,
            &BackupOptions::default(),
        )
        .unwrap();

        // The incremental backup must contain nodes 10 and 11 but NOT old_node (0).
        assert!(
            meta.changed_node_ids.contains(&10),
            "expected node 10 in changed list"
        );
        assert!(
            meta.changed_node_ids.contains(&11),
            "expected node 11 in changed list"
        );
        assert!(
            !meta.changed_node_ids.contains(&old_node),
            "old_node should NOT appear in incremental"
        );
        assert_eq!(meta.base_backup_id, "base-001");

        std::fs::remove_file(&wal_path).ok();
        std::fs::remove_file(&base_backup_path).ok();
        std::fs::remove_file(&incr_path).ok();
    }

    #[test]
    fn test_restore_incremental_matches_full_state() {
        use crate::wal::{RecordPayload, RecordType, Wal};

        let base_ts: u64 = 2_000_000;
        let base_backup_path = tmp_path("incr_restore_base.backup");
        let incr_path = tmp_path("incr_restore_delta.ibackup");
        let wal_path = tmp_path("incr_restore.wal");

        // Step 1: base graph (just one node).
        let mut graph = Graph::new();
        let base_node = graph.create_node("Base"); // id = 0
        Backup::create(&graph, &base_backup_path, &BackupOptions::default()).unwrap();

        // Step 2: more changes after base_ts (node 10 added, node 0 deleted).
        graph.create_node_with_id(10, "NewNode");
        if let Some(n) = graph.get_node_mut(10) {
            n.set_property("x", 99i64);
        }
        graph.delete_node(base_node);

        // Step 3: WAL reflecting those changes.
        let mut wal = Wal::open(&wal_path).unwrap();
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 10,
                label: "NewNode".to_string(),
            },
            base_ts + 1,
        )
        .unwrap();
        wal.append_at(
            RecordType::SetNodeProperty,
            RecordPayload::SetNodeProperty {
                node_id: 10,
                key: "x".to_string(),
                value: maharit_core::PropertyValue::Int(99),
            },
            base_ts + 2,
        )
        .unwrap();
        wal.append_at(
            RecordType::DeleteNode,
            RecordPayload::DeleteNode { node_id: 0 },
            base_ts + 3,
        )
        .unwrap();
        wal.sync().unwrap();

        // Step 4: create incremental backup.
        Backup::create_incremental(
            &graph,
            "base-001",
            base_ts,
            &wal_path,
            &incr_path,
            &BackupOptions::default(),
        )
        .unwrap();

        // Step 5: restore incremental and compare with the current graph.
        let restored = Backup::restore_incremental(&base_backup_path, &incr_path).unwrap();

        // The restored graph should match `graph` — one node (id 10), base_node deleted.
        assert_eq!(
            restored.node_count(),
            graph.node_count(),
            "node count mismatch after incremental restore"
        );
        assert!(
            restored.get_node(base_node).is_none(),
            "base_node should have been deleted"
        );
        assert!(
            restored.get_node(10).is_some(),
            "new node 10 should be present"
        );
        assert_eq!(
            restored.get_node(10).unwrap().properties.get("x"),
            Some(&maharit_core::PropertyValue::Int(99))
        );

        std::fs::remove_file(&base_backup_path).ok();
        std::fs::remove_file(&incr_path).ok();
        std::fs::remove_file(&wal_path).ok();
    }

    #[test]
    fn test_pitr_restores_to_specific_timestamp() {
        use crate::wal::{RecordPayload, RecordType, Wal};

        let base_ts: u64 = 3_000_000;
        let base_backup_path = tmp_path("pitr_base.backup");
        let wal_path = tmp_path("pitr_test.wal");

        // Base graph: empty.
        let graph = Graph::new();
        Backup::create(&graph, &base_backup_path, &BackupOptions::default()).unwrap();

        // WAL: three operations at t+1, t+2, t+3.
        let mut wal = Wal::open(&wal_path).unwrap();
        // t+1: create node 0 "Alpha"
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "Alpha".to_string(),
            },
            base_ts + 1,
        )
        .unwrap();
        // t+2: create node 1 "Beta"
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 1,
                label: "Beta".to_string(),
            },
            base_ts + 2,
        )
        .unwrap();
        // t+3: delete node 0
        wal.append_at(
            RecordType::DeleteNode,
            RecordPayload::DeleteNode { node_id: 0 },
            base_ts + 3,
        )
        .unwrap();
        wal.sync().unwrap();

        // PITR to t+2 (after Alpha and Beta created, before Alpha deleted).
        let restored_t2 =
            Backup::restore_to_point_in_time(&base_backup_path, &wal_path, base_ts + 2).unwrap();
        assert_eq!(restored_t2.node_count(), 2, "expected both nodes at t+2");

        // PITR to t+3 (Alpha should be gone).
        let restored_t3 =
            Backup::restore_to_point_in_time(&base_backup_path, &wal_path, base_ts + 3).unwrap();
        assert_eq!(restored_t3.node_count(), 1, "expected only Beta at t+3");

        // PITR to future timestamp (should give same as t+3, i.e. latest state).
        let restored_future = Backup::restore_to_point_in_time(
            &base_backup_path,
            &wal_path,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(
            restored_future.node_count(),
            1,
            "future timestamp should give latest state"
        );

        std::fs::remove_file(&base_backup_path).ok();
        std::fs::remove_file(&wal_path).ok();
    }

    #[test]
    fn test_pitr_future_timestamp_gives_latest_state() {
        use crate::wal::{RecordPayload, RecordType, Wal};

        let base_ts: u64 = 4_000_000;
        let base_backup_path = tmp_path("pitr_future_base.backup");
        let wal_path = tmp_path("pitr_future.wal");

        let graph = Graph::new();
        Backup::create(&graph, &base_backup_path, &BackupOptions::default()).unwrap();

        let mut wal = Wal::open(&wal_path).unwrap();
        wal.append_at(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "Node".to_string(),
            },
            base_ts + 5,
        )
        .unwrap();
        wal.sync().unwrap();

        // Future timestamp: should include the node created at base_ts+5.
        let restored =
            Backup::restore_to_point_in_time(&base_backup_path, &wal_path, u64::MAX).unwrap();
        assert_eq!(restored.node_count(), 1);

        std::fs::remove_file(&base_backup_path).ok();
        std::fs::remove_file(&wal_path).ok();
    }

    #[test]
    fn test_backup_restore_with_index_definitions() {
        use maharit_core::{IndexDefinition, PropertyIndex};

        // Create a graph with nodes
        let mut graph = Graph::new();
        let n1 = graph.create_node_with_labels(vec!["Person".to_string()]);
        graph
            .get_node_mut(n1)
            .unwrap()
            .set_property("name", PropertyValue::String("Alice".to_string()));
        let n2 = graph.create_node_with_labels(vec!["Person".to_string()]);
        graph
            .get_node_mut(n2)
            .unwrap()
            .set_property("name", PropertyValue::String("Bob".to_string()));

        // Create a PropertyIndex with one definition
        let mut index = PropertyIndex::new();
        index.create_index(IndexDefinition::new("Person", "name"));
        // Index existing nodes
        for node_id in [n1, n2] {
            let val = graph
                .get_node(node_id)
                .unwrap()
                .get_property("name")
                .unwrap()
                .clone();
            index.index_property(node_id, "name", &val);
        }

        let path = tmp_path("test_backup_with_index.db");

        // Backup with index
        Backup::create_with_index(&graph, &index, &path, &BackupOptions::default()).unwrap();

        // Restore with index
        let (restored_graph, restored_index) = Backup::restore_with_index(&path).unwrap();
        assert_eq!(restored_graph.node_count(), 2);

        // Index definitions should be restored
        let defs = restored_index.list_indexes();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].label, "Person");
        assert_eq!(defs[0].property, "name");

        // Index data should be queryable
        let alice_nodes = restored_index
            .find_by_property("name", &PropertyValue::String("Alice".to_string()));
        assert!(!alice_nodes.is_empty());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_restore_old_format_without_index_section() {

        // Restore without index should work (returns empty index)
        let mut graph = Graph::new();
        graph.create_node_with_labels(vec!["Node".to_string()]);

        let path = tmp_path("test_backup_no_index.db");

        // create() does not include index section
        Backup::create(&graph, &path, &BackupOptions::default()).unwrap();

        // restore_with_index should still work, returning empty PropertyIndex
        let (restored_graph, restored_index) = Backup::restore_with_index(&path).unwrap();
        assert_eq!(restored_graph.node_count(), 1);
        assert_eq!(restored_index.list_indexes().len(), 0);

        std::fs::remove_file(&path).ok();
    }
}
