use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use maharit_core::{Graph, PropertyValue};
use thiserror::Error;

/// Log Sequence Number
pub type Lsn = u64;

/// WALエラー
#[derive(Debug, Error)]
pub enum WalError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("corrupted log: {0}")]
    CorruptedLog(String),

    #[error("checksum mismatch")]
    ChecksumMismatch,

    #[error("graph error: {0}")]
    Graph(#[from] maharit_core::GraphError),
}

pub type Result<T> = std::result::Result<T, WalError>;

/// ログレコードの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordType {
    CreateNode = 1,
    DeleteNode = 2,
    CreateEdge = 3,
    DeleteEdge = 4,
    SetNodeProperty = 5,
    SetEdgeProperty = 6,
    Checkpoint = 7,
}

impl TryFrom<u8> for RecordType {
    type Error = WalError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(RecordType::CreateNode),
            2 => Ok(RecordType::DeleteNode),
            3 => Ok(RecordType::CreateEdge),
            4 => Ok(RecordType::DeleteEdge),
            5 => Ok(RecordType::SetNodeProperty),
            6 => Ok(RecordType::SetEdgeProperty),
            7 => Ok(RecordType::Checkpoint),
            _ => Err(WalError::CorruptedLog(format!(
                "unknown record type: {}",
                value
            ))),
        }
    }
}

/// ログレコード
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub lsn: Lsn,
    pub timestamp: u64,
    pub record_type: RecordType,
    pub payload: RecordPayload,
}

/// レコードのペイロード
#[derive(Debug, Clone)]
pub enum RecordPayload {
    CreateNode {
        node_id: u64,
        label: String,
    },
    DeleteNode {
        node_id: u64,
    },
    CreateEdge {
        edge_id: u64,
        from: u64,
        to: u64,
        label: String,
    },
    DeleteEdge {
        edge_id: u64,
    },
    SetNodeProperty {
        node_id: u64,
        key: String,
        value: PropertyValue,
    },
    SetEdgeProperty {
        edge_id: u64,
        key: String,
        value: PropertyValue,
    },
    Checkpoint {
        last_lsn: Lsn,
    },
}

/// Write-Ahead Log
pub struct Wal {
    path: PathBuf,
    writer: BufWriter<File>,
    current_lsn: Lsn,
}

impl Wal {
    /// 新しいWALを作成または既存のWALを開く
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // 既存のログからLSNを復元
        let current_lsn = Self::find_last_lsn(&path)?;

        let writer = BufWriter::new(file);

        Ok(Self {
            path,
            writer,
            current_lsn,
        })
    }

    /// レコードを追記
    pub fn append(&mut self, record_type: RecordType, payload: RecordPayload) -> Result<Lsn> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.append_at(record_type, payload, timestamp)
    }

    /// テスト用: カスタムタイムスタンプでレコードを追記
    ///
    /// 本番コードでは `append` を使用すること。
    pub fn append_at(
        &mut self,
        record_type: RecordType,
        payload: RecordPayload,
        timestamp: u64,
    ) -> Result<Lsn> {
        self.current_lsn += 1;
        let lsn = self.current_lsn;

        let record = LogRecord {
            lsn,
            timestamp,
            record_type,
            payload,
        };

        self.write_record(&record)?;

        Ok(lsn)
    }

    /// バッファをフラッシュしてディスクに同期
    pub fn sync(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    /// ログからグラフを復元
    pub fn recover(&self, graph: &mut Graph) -> Result<Lsn> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);

        let mut last_lsn = 0;
        let mut id_map: HashMap<u64, u64> = HashMap::new(); // old_id -> new_id

        loop {
            match Self::read_record(&mut reader) {
                Ok(Some(record)) => {
                    last_lsn = record.lsn;
                    Self::apply_record(graph, &record, &mut id_map)?;
                }
                Ok(None) => break,
                Err(WalError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
        }

        Ok(last_lsn)
    }

    /// チェックポイントを作成
    pub fn checkpoint(&mut self) -> Result<Lsn> {
        let last_lsn = self.current_lsn;
        self.append(
            RecordType::Checkpoint,
            RecordPayload::Checkpoint { last_lsn },
        )
    }

    /// 現在のLSN
    pub fn current_lsn(&self) -> Lsn {
        self.current_lsn
    }

    /// 古いログを削除（最後のチェックポイント以前のレコードを削除）
    ///
    /// チェックポイントがない場合は何もしない。
    /// 返り値: 削除されたレコード数
    pub fn cleanup(&mut self) -> Result<usize> {
        // Flush current buffer first
        self.sync()?;

        // Read all records and find the last checkpoint
        let (records, last_checkpoint_lsn) = self.read_all_records()?;

        // If no checkpoint, nothing to clean up
        let checkpoint_lsn = match last_checkpoint_lsn {
            Some(lsn) => lsn,
            None => return Ok(0),
        };

        // Count how many records to remove
        let records_before = records
            .iter()
            .take_while(|r| r.lsn <= checkpoint_lsn)
            .count();
        if records_before == 0 {
            return Ok(0);
        }

        // Keep records after the checkpoint
        let records_to_keep: Vec<_> = records
            .into_iter()
            .filter(|r| r.lsn > checkpoint_lsn)
            .collect();

        // Create a new temporary WAL file
        let tmp_path = self.path.with_extension("wal.tmp");

        {
            let tmp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)?;

            let mut tmp_writer = BufWriter::new(tmp_file);

            // Write kept records to the temp file
            for record in &records_to_keep {
                Self::write_record_to(&mut tmp_writer, record)?;
            }

            tmp_writer.flush()?;
            tmp_writer.get_ref().sync_all()?;
        }

        // Replace old file with new one
        std::fs::rename(&tmp_path, &self.path)?;

        // Reopen the WAL file
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .truncate(false)
            .open(&self.path)?;

        self.writer = BufWriter::new(file);

        Ok(records_before)
    }

    /// ログファイルのサイズを取得（バイト単位）
    pub fn file_size(&self) -> Result<u64> {
        let metadata = std::fs::metadata(&self.path)?;
        Ok(metadata.len())
    }

    /// レコード数を取得
    pub fn record_count(&self) -> Result<usize> {
        let (records, _) = self.read_all_records()?;
        Ok(records.len())
    }

    // ========== Internal methods ==========

    /// Read all WAL records whose `timestamp` is strictly greater than
    /// `after_timestamp`, in LSN order.  Returns the filtered records and the
    /// optional LSN of the last checkpoint record encountered (regardless of
    /// its timestamp).
    ///
    /// Pass `after_timestamp = 0` to retrieve every record in the log.
    pub fn read_all_for_incremental(
        &self,
        after_timestamp: u64,
    ) -> (Vec<LogRecord>, Option<Lsn>) {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return (Vec::new(), None),
        };
        let mut reader = BufReader::new(file);

        let mut matching_records = Vec::new();
        let mut last_checkpoint_lsn = None;

        loop {
            match Self::read_record(&mut reader) {
                Ok(Some(record)) => {
                    if record.record_type == RecordType::Checkpoint {
                        last_checkpoint_lsn = Some(record.lsn);
                    }
                    if record.timestamp > after_timestamp {
                        matching_records.push(record);
                    }
                }
                Ok(None) => break,
                Err(WalError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(_) => break,
            }
        }

        (matching_records, last_checkpoint_lsn)
    }

    /// Read all records and find the last checkpoint record's LSN
    fn read_all_records(&self) -> Result<(Vec<LogRecord>, Option<Lsn>)> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);

        let mut records = Vec::new();
        let mut last_checkpoint_record_lsn = None;

        loop {
            match Self::read_record(&mut reader) {
                Ok(Some(record)) => {
                    if record.record_type == RecordType::Checkpoint {
                        // Store the checkpoint record's own LSN (not the last_lsn inside it)
                        last_checkpoint_record_lsn = Some(record.lsn);
                    }
                    records.push(record);
                }
                Ok(None) => break,
                Err(WalError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
        }

        Ok((records, last_checkpoint_record_lsn))
    }

    /// Write a record to a writer
    fn write_record_to<W: Write>(writer: &mut W, record: &LogRecord) -> Result<()> {
        // LSN
        writer.write_all(&record.lsn.to_le_bytes())?;
        // Timestamp
        writer.write_all(&record.timestamp.to_le_bytes())?;
        // Record type
        writer.write_all(&[record.record_type as u8])?;

        // Payload
        match &record.payload {
            RecordPayload::CreateNode { node_id, label } => {
                writer.write_all(&node_id.to_le_bytes())?;
                Self::write_string(writer, label)?;
            }
            RecordPayload::DeleteNode { node_id } => {
                writer.write_all(&node_id.to_le_bytes())?;
            }
            RecordPayload::CreateEdge {
                edge_id,
                from,
                to,
                label,
            } => {
                writer.write_all(&edge_id.to_le_bytes())?;
                writer.write_all(&from.to_le_bytes())?;
                writer.write_all(&to.to_le_bytes())?;
                Self::write_string(writer, label)?;
            }
            RecordPayload::DeleteEdge { edge_id } => {
                writer.write_all(&edge_id.to_le_bytes())?;
            }
            RecordPayload::SetNodeProperty {
                node_id,
                key,
                value,
            } => {
                writer.write_all(&node_id.to_le_bytes())?;
                Self::write_string(writer, key)?;
                Self::write_property(writer, value)?;
            }
            RecordPayload::SetEdgeProperty {
                edge_id,
                key,
                value,
            } => {
                writer.write_all(&edge_id.to_le_bytes())?;
                Self::write_string(writer, key)?;
                Self::write_property(writer, value)?;
            }
            RecordPayload::Checkpoint { last_lsn } => {
                writer.write_all(&last_lsn.to_le_bytes())?;
            }
        }

        // Checksum
        let checksum = record.lsn ^ record.timestamp;
        writer.write_all(&checksum.to_le_bytes())?;

        Ok(())
    }

    fn find_last_lsn(path: &Path) -> Result<Lsn> {
        if !path.exists() {
            return Ok(0);
        }

        let file = File::open(path)?;
        let metadata = file.metadata()?;

        if metadata.len() == 0 {
            return Ok(0);
        }

        let mut reader = BufReader::new(file);
        let mut last_lsn = 0;

        loop {
            match Self::read_record(&mut reader) {
                Ok(Some(record)) => {
                    last_lsn = record.lsn;
                }
                Ok(None) => break,
                Err(WalError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(_) => break,
            }
        }

        Ok(last_lsn)
    }

    fn write_record(&mut self, record: &LogRecord) -> Result<()> {
        // LSN
        self.writer.write_all(&record.lsn.to_le_bytes())?;
        // Timestamp
        self.writer.write_all(&record.timestamp.to_le_bytes())?;
        // Record type
        self.writer.write_all(&[record.record_type as u8])?;

        // Payload
        match &record.payload {
            RecordPayload::CreateNode { node_id, label } => {
                self.writer.write_all(&node_id.to_le_bytes())?;
                Self::write_string(&mut self.writer, label)?;
            }
            RecordPayload::DeleteNode { node_id } => {
                self.writer.write_all(&node_id.to_le_bytes())?;
            }
            RecordPayload::CreateEdge {
                edge_id,
                from,
                to,
                label,
            } => {
                self.writer.write_all(&edge_id.to_le_bytes())?;
                self.writer.write_all(&from.to_le_bytes())?;
                self.writer.write_all(&to.to_le_bytes())?;
                Self::write_string(&mut self.writer, label)?;
            }
            RecordPayload::DeleteEdge { edge_id } => {
                self.writer.write_all(&edge_id.to_le_bytes())?;
            }
            RecordPayload::SetNodeProperty {
                node_id,
                key,
                value,
            } => {
                self.writer.write_all(&node_id.to_le_bytes())?;
                Self::write_string(&mut self.writer, key)?;
                Self::write_property(&mut self.writer, value)?;
            }
            RecordPayload::SetEdgeProperty {
                edge_id,
                key,
                value,
            } => {
                self.writer.write_all(&edge_id.to_le_bytes())?;
                Self::write_string(&mut self.writer, key)?;
                Self::write_property(&mut self.writer, value)?;
            }
            RecordPayload::Checkpoint { last_lsn } => {
                self.writer.write_all(&last_lsn.to_le_bytes())?;
            }
        }

        // Simple checksum (XOR of all bytes would be better, but this is simple)
        let checksum = record.lsn ^ record.timestamp;
        self.writer.write_all(&checksum.to_le_bytes())?;

        Ok(())
    }

    fn read_record<R: Read>(reader: &mut R) -> Result<Option<LogRecord>> {
        // LSN
        let mut buf = [0u8; 8];
        match reader.read_exact(&mut buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let lsn = u64::from_le_bytes(buf);

        // Timestamp
        reader.read_exact(&mut buf)?;
        let timestamp = u64::from_le_bytes(buf);

        // Record type
        let mut type_buf = [0u8; 1];
        reader.read_exact(&mut type_buf)?;
        let record_type = RecordType::try_from(type_buf[0])?;

        // Payload
        let payload = match record_type {
            RecordType::CreateNode => {
                reader.read_exact(&mut buf)?;
                let node_id = u64::from_le_bytes(buf);
                let label = Self::read_string(reader)?;
                RecordPayload::CreateNode { node_id, label }
            }
            RecordType::DeleteNode => {
                reader.read_exact(&mut buf)?;
                let node_id = u64::from_le_bytes(buf);
                RecordPayload::DeleteNode { node_id }
            }
            RecordType::CreateEdge => {
                reader.read_exact(&mut buf)?;
                let edge_id = u64::from_le_bytes(buf);
                reader.read_exact(&mut buf)?;
                let from = u64::from_le_bytes(buf);
                reader.read_exact(&mut buf)?;
                let to = u64::from_le_bytes(buf);
                let label = Self::read_string(reader)?;
                RecordPayload::CreateEdge {
                    edge_id,
                    from,
                    to,
                    label,
                }
            }
            RecordType::DeleteEdge => {
                reader.read_exact(&mut buf)?;
                let edge_id = u64::from_le_bytes(buf);
                RecordPayload::DeleteEdge { edge_id }
            }
            RecordType::SetNodeProperty => {
                reader.read_exact(&mut buf)?;
                let node_id = u64::from_le_bytes(buf);
                let key = Self::read_string(reader)?;
                let value = Self::read_property(reader)?;
                RecordPayload::SetNodeProperty {
                    node_id,
                    key,
                    value,
                }
            }
            RecordType::SetEdgeProperty => {
                reader.read_exact(&mut buf)?;
                let edge_id = u64::from_le_bytes(buf);
                let key = Self::read_string(reader)?;
                let value = Self::read_property(reader)?;
                RecordPayload::SetEdgeProperty {
                    edge_id,
                    key,
                    value,
                }
            }
            RecordType::Checkpoint => {
                reader.read_exact(&mut buf)?;
                let last_lsn = u64::from_le_bytes(buf);
                RecordPayload::Checkpoint { last_lsn }
            }
        };

        // Checksum
        reader.read_exact(&mut buf)?;
        let stored_checksum = u64::from_le_bytes(buf);
        let computed_checksum = lsn ^ timestamp;
        if stored_checksum != computed_checksum {
            return Err(WalError::ChecksumMismatch);
        }

        Ok(Some(LogRecord {
            lsn,
            timestamp,
            record_type,
            payload,
        }))
    }

    fn apply_record(
        graph: &mut Graph,
        record: &LogRecord,
        id_map: &mut HashMap<u64, u64>,
    ) -> Result<()> {
        match &record.payload {
            RecordPayload::CreateNode { node_id, label } => {
                let new_id = graph.create_node(label);
                id_map.insert(*node_id, new_id);
            }
            RecordPayload::DeleteNode { node_id } => {
                if let Some(&actual_id) = id_map.get(node_id) {
                    graph.delete_node(actual_id);
                }
            }
            RecordPayload::CreateEdge {
                edge_id: _,
                from,
                to,
                label,
            } => {
                let actual_from = id_map.get(from).copied().unwrap_or(*from);
                let actual_to = id_map.get(to).copied().unwrap_or(*to);
                graph.create_edge(actual_from, actual_to, label)?;
            }
            RecordPayload::DeleteEdge { edge_id } => {
                graph.delete_edge(*edge_id);
            }
            RecordPayload::SetNodeProperty {
                node_id,
                key,
                value,
            } => {
                let actual_id = id_map.get(node_id).copied().unwrap_or(*node_id);
                if let Some(node) = graph.get_node_mut(actual_id) {
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
            RecordPayload::Checkpoint { .. } => {
                // Checkpoint is informational only
            }
        }

        Ok(())
    }

    fn write_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
        let bytes = s.as_bytes();
        let len = bytes.len() as u32;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(bytes)?;
        Ok(())
    }

    fn read_string<R: Read>(reader: &mut R) -> Result<String> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;

        String::from_utf8(buf).map_err(|_| WalError::CorruptedLog("invalid UTF-8".to_string()))
    }

    fn write_property<W: Write>(writer: &mut W, value: &PropertyValue) -> Result<()> {
        match value {
            PropertyValue::Null => writer.write_all(&[0u8])?,
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

    fn read_property<R: Read>(reader: &mut R) -> Result<PropertyValue> {
        let mut type_buf = [0u8; 1];
        reader.read_exact(&mut type_buf)?;

        match type_buf[0] {
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
            t => Err(WalError::CorruptedLog(format!(
                "unknown property type: {}",
                t
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        let id = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(format!("/tmp/maharit_wal_test_{}.wal", id))
    }

    #[test]
    fn test_create_and_recover() {
        let path = temp_path();

        // Write some records
        {
            let mut wal = Wal::open(&path).unwrap();

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "Person".to_string(),
                },
            )
            .unwrap();

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 1,
                    label: "Person".to_string(),
                },
            )
            .unwrap();

            wal.append(
                RecordType::CreateEdge,
                RecordPayload::CreateEdge {
                    edge_id: 0,
                    from: 0,
                    to: 1,
                    label: "KNOWS".to_string(),
                },
            )
            .unwrap();

            wal.sync().unwrap();
        }

        // Recover
        {
            let wal = Wal::open(&path).unwrap();
            let mut graph = Graph::new();

            let last_lsn = wal.recover(&mut graph).unwrap();

            assert_eq!(last_lsn, 3);
            assert_eq!(graph.node_count(), 2);
            assert_eq!(graph.edge_count(), 1);
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_set_property() {
        let path = temp_path();

        {
            let mut wal = Wal::open(&path).unwrap();

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "Person".to_string(),
                },
            )
            .unwrap();

            wal.append(
                RecordType::SetNodeProperty,
                RecordPayload::SetNodeProperty {
                    node_id: 0,
                    key: "name".to_string(),
                    value: PropertyValue::String("Alice".to_string()),
                },
            )
            .unwrap();

            wal.sync().unwrap();
        }

        {
            let wal = Wal::open(&path).unwrap();
            let mut graph = Graph::new();

            wal.recover(&mut graph).unwrap();

            let node = graph.nodes().next().unwrap();
            assert_eq!(
                node.properties.get("name"),
                Some(&PropertyValue::String("Alice".to_string()))
            );
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_checkpoint() {
        let path = temp_path();

        {
            let mut wal = Wal::open(&path).unwrap();

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "Test".to_string(),
                },
            )
            .unwrap();

            let cp_lsn = wal.checkpoint().unwrap();
            assert_eq!(cp_lsn, 2);

            wal.sync().unwrap();
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_lsn_persistence() {
        let path = temp_path();

        // First session
        {
            let mut wal = Wal::open(&path).unwrap();
            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 0,
                    label: "A".to_string(),
                },
            )
            .unwrap();
            wal.sync().unwrap();
            assert_eq!(wal.current_lsn(), 1);
        }

        // Second session
        {
            let mut wal = Wal::open(&path).unwrap();
            assert_eq!(wal.current_lsn(), 1);

            wal.append(
                RecordType::CreateNode,
                RecordPayload::CreateNode {
                    node_id: 1,
                    label: "B".to_string(),
                },
            )
            .unwrap();
            wal.sync().unwrap();
            assert_eq!(wal.current_lsn(), 2);
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_cleanup_without_checkpoint() {
        let path = temp_path();

        let mut wal = Wal::open(&path).unwrap();

        // Add some records without checkpoint
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "A".to_string(),
            },
        )
        .unwrap();
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 1,
                label: "B".to_string(),
            },
        )
        .unwrap();
        wal.sync().unwrap();

        // Cleanup should do nothing without checkpoint
        let removed = wal.cleanup().unwrap();
        assert_eq!(removed, 0);
        assert_eq!(wal.record_count().unwrap(), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_cleanup_with_checkpoint() {
        let path = temp_path();

        let mut wal = Wal::open(&path).unwrap();

        // Add records before checkpoint
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "A".to_string(),
            },
        )
        .unwrap();
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 1,
                label: "B".to_string(),
            },
        )
        .unwrap();

        // Create checkpoint
        wal.checkpoint().unwrap();

        // Add records after checkpoint
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 2,
                label: "C".to_string(),
            },
        )
        .unwrap();
        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 3,
                label: "D".to_string(),
            },
        )
        .unwrap();
        wal.sync().unwrap();

        assert_eq!(wal.record_count().unwrap(), 5); // 2 + checkpoint + 2

        // Cleanup should remove records up to and including checkpoint
        let removed = wal.cleanup().unwrap();
        assert_eq!(removed, 3); // 2 records + checkpoint

        // Only 2 records should remain
        assert_eq!(wal.record_count().unwrap(), 2);

        // Verify recovery still works with the cleaned up log
        let mut graph = Graph::new();
        wal.recover(&mut graph).unwrap();
        // The recovered graph will only have nodes C and D
        assert_eq!(graph.node_count(), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_file_size() {
        let path = temp_path();

        let mut wal = Wal::open(&path).unwrap();

        wal.append(
            RecordType::CreateNode,
            RecordPayload::CreateNode {
                node_id: 0,
                label: "Test".to_string(),
            },
        )
        .unwrap();
        wal.sync().unwrap();

        let size = wal.file_size().unwrap();
        assert!(size > 0);

        std::fs::remove_file(&path).ok();
    }
}
