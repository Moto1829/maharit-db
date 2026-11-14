use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use maharit_core::{ConcurrentGraph, Graph, IndexDefinition, PropertyIndex, PropertyValue};
use thiserror::Error;

use crate::snapshot_codec::{self, SEdge, SIndex, SNode, SSnapshot};

/// ファイルフォーマットのマジックナンバー
const MAGIC: &[u8; 8] = b"MAHARITD";
/// v6: v5 と同じ並びで、プロパティ値にリストを含むもの。リストが無ければ v4 / v5 で書くので、
/// リストを使わない限り旧版でも読める
const VERSION_V6: u32 = 6;
/// v5: v4 のペイロードの後ろにスキーマ（インデックス・制約の定義、JSON 文字列）。
/// `save_concurrent_with_schema` が書く
const VERSION_V5: u32 = 5;
/// v4: bincode 1 互換のスナップショット（`snapshot_codec` 参照）。`save_with_index` が書く
const VERSION: u32 = 4;
/// v3: ラベルを Vec<String> として保存
const VERSION_V3: u32 = 3;
/// v2: インデックスセクション追加
const VERSION_V2: u32 = 2;
/// 後方互換性のため v1 も読み込み可能にする
const VERSION_V1: u32 = 1;
/// BufWriter バッファサイズ（4MB）
const BUF_CAPACITY: usize = 4 * 1024 * 1024;

/// 永続化エラー
#[derive(Debug, Error)]
pub enum PersistenceError {
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

pub type Result<T> = std::result::Result<T, PersistenceError>;

/// 永続化ストレージ
pub struct PersistentStorage;

impl PersistentStorage {
    /// グラフをファイルに保存（インデックスなし）
    pub fn save(graph: &Graph, path: impl AsRef<Path>) -> Result<()> {
        let empty_index = PropertyIndex::new();
        Self::save_with_index(graph, &empty_index, path)
    }

    /// グラフとプロパティインデックスをファイルに保存（v4 フォーマット）
    pub fn save_with_index(
        graph: &Graph,
        index: &PropertyIndex,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        // Build serializable snapshot
        let nodes: Vec<SNode> = graph
            .nodes()
            .map(|n| SNode {
                id: n.id,
                labels: n.labels.clone(),
                properties: n.properties.as_ref().clone(),
            })
            .collect();

        let edges: Vec<SEdge> = graph
            .edges()
            .map(|e| SEdge {
                id: e.id,
                label: e.label.clone(),
                from: e.from,
                to: e.to,
                properties: e.properties.as_ref().clone(),
            })
            .collect();

        let indexes: Vec<SIndex> = index
            .list_indexes()
            .into_iter()
            .map(|def| SIndex {
                label: def.label.clone(),
                property: def.property.clone(),
                unique: def.unique,
            })
            .collect();

        let snapshot = SSnapshot {
            nodes,
            edges,
            indexes,
        };

        if snapshot.has_lists() {
            // v4 にはリストを書けない。v6（v5 の並び、スキーマは空）で書く
            let encoded = snapshot_codec::encode_v5(&snapshot, "");
            return Self::write_snapshot_file(path.as_ref(), VERSION_V6, &encoded);
        }
        let encoded = snapshot_codec::encode(&snapshot);

        Self::write_snapshot_file(path.as_ref(), VERSION, &encoded)
    }

    /// ヘッダー + 長さ付きペイロードを `path` に原子的に書き込む。
    ///
    /// 一時ファイル（`<path>.tmp`）に書いて fsync してから rename で置き換え、
    /// ディレクトリも fsync する。保存中にプロセスが落ちても、元のファイルか
    /// 新しいファイルのどちらかが完全な形で残る（途中まで書かれたファイルにならない）。
    fn write_snapshot_file(path: &Path, version: u32, encoded: &[u8]) -> Result<()> {
        let tmp = {
            let mut s = path.as_os_str().to_owned();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        {
            let file = File::create(&tmp)?;
            let mut writer = BufWriter::with_capacity(BUF_CAPACITY, file);

            // Header: magic + version
            writer.write_all(MAGIC)?;
            writer.write_all(&version.to_le_bytes())?;

            // Length-prefixed payload
            let payload_len = encoded.len() as u64;
            writer.write_all(&payload_len.to_le_bytes())?;
            writer.write_all(encoded)?;

            writer.flush()?;
            writer.get_ref().sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        {
            let dir = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            File::open(dir)?.sync_all()?;
        }
        Ok(())
    }

    /// ConcurrentGraph をファイルに保存（スキーマなし）
    pub fn save_concurrent(graph: &ConcurrentGraph, path: impl AsRef<Path>) -> Result<()> {
        Self::save_concurrent_with_schema(graph, "", path)
    }

    /// ConcurrentGraph とスキーマ（呼び出し側が決める文字列。空ならなし）を
    /// v5 形式で原子的に保存する
    pub fn save_concurrent_with_schema(
        graph: &ConcurrentGraph,
        schema: &str,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let nodes: Vec<SNode> = graph
            .nodes()
            .map(|r| {
                let n = r.value();
                SNode {
                    id: n.id,
                    labels: n.labels.clone(),
                    properties: n.properties.as_ref().clone(),
                }
            })
            .collect();

        let edges: Vec<SEdge> = graph
            .edges()
            .map(|r| {
                let e = r.value();
                SEdge {
                    id: e.id,
                    label: e.label.clone(),
                    from: e.from,
                    to: e.to,
                    properties: e.properties.as_ref().clone(),
                }
            })
            .collect();

        let snapshot = SSnapshot {
            nodes,
            edges,
            indexes: vec![],
        };

        let encoded = snapshot_codec::encode_v5(&snapshot, schema);
        let version = if snapshot.has_lists() {
            VERSION_V6
        } else {
            VERSION_V5
        };

        Self::write_snapshot_file(path.as_ref(), version, &encoded)
    }

    /// ファイルから ConcurrentGraph を読み込み（スキーマは捨てる）
    pub fn load_concurrent(path: impl AsRef<Path>) -> Result<ConcurrentGraph> {
        Self::load_concurrent_with_schema(path).map(|(graph, _)| graph)
    }

    /// ファイルから ConcurrentGraph とスキーマ文字列を読み込む。v4 のファイルは
    /// スキーマを持たないので `None`、v5 で空文字列のときも `None`
    pub fn load_concurrent_with_schema(
        path: impl AsRef<Path>,
    ) -> Result<(ConcurrentGraph, Option<String>)> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(PersistenceError::InvalidMagic);
        }

        let version = Self::read_u32(&mut reader)?;
        let (snapshot, schema) = match version {
            VERSION => (Self::read_snapshot(&mut reader)?, None),
            VERSION_V5 | VERSION_V6 => {
                let (snapshot, schema) = Self::read_snapshot_v5(&mut reader)?;
                (snapshot, Some(schema).filter(|s| !s.is_empty()))
            }
            other => return Err(PersistenceError::UnsupportedVersion(other)),
        };

        let graph = ConcurrentGraph::new();

        // Keep the saved IDs. The server's WAL addresses nodes and edges by ID,
        // so a snapshot taken while running must reload with the same IDs for
        // the WAL written after it to replay correctly.
        for snode in snapshot.nodes {
            if graph.contains_node(snode.id) {
                return Err(PersistenceError::CorruptedData(format!(
                    "duplicate node id: {}",
                    snode.id
                )));
            }
            let id = graph.create_node_with_id_and_labels(snode.id, snode.labels);
            for (k, v) in snode.properties {
                graph.set_node_property(id, &k, v);
            }
        }

        for sedge in snapshot.edges {
            for end in [sedge.from, sedge.to] {
                if !graph.contains_node(end) {
                    return Err(PersistenceError::CorruptedData(format!(
                        "unknown node id: {}",
                        end
                    )));
                }
            }
            let edge_id =
                graph.create_edge_with_id(sedge.id, sedge.from, sedge.to, &sedge.label)?;
            for (k, v) in sedge.properties {
                graph.set_edge_property(edge_id, &k, v);
            }
        }

        Ok((graph, schema))
    }

    /// Graph とスキーマ文字列を v5 形式で原子的に保存する（`maharit restore` 用）
    pub fn save_with_schema(graph: &Graph, schema: &str, path: impl AsRef<Path>) -> Result<()> {
        let concurrent = ConcurrentGraph::new();
        for node in graph.nodes() {
            concurrent.create_node_with_id_and_labels(node.id, node.labels.clone());
            for (k, v) in node.properties.iter() {
                concurrent.set_node_property(node.id, k, v.clone());
            }
        }
        for edge in graph.edges() {
            concurrent.create_edge_with_id(edge.id, edge.from, edge.to, &edge.label)?;
            for (k, v) in edge.properties.iter() {
                concurrent.set_edge_property(edge.id, k, v.clone());
            }
        }
        Self::save_concurrent_with_schema(&concurrent, schema, path)
    }

    /// ファイルからグラフとスキーマ文字列を読み込む（v5 以外は `None`）
    pub fn load_with_schema(path: impl AsRef<Path>) -> Result<(Graph, Option<String>)> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(PersistenceError::InvalidMagic);
        }
        if matches!(Self::read_u32(&mut reader)?, VERSION_V5 | VERSION_V6) {
            let (snapshot, schema) = Self::read_snapshot_v5(&mut reader)?;
            let (graph, _) = Self::graph_from_snapshot(snapshot)?;
            return Ok((graph, Some(schema).filter(|s| !s.is_empty())));
        }
        Ok((Self::load(path)?, None))
    }

    /// ファイルからグラフを読み込み（インデックスは無視）
    pub fn load(path: impl AsRef<Path>) -> Result<Graph> {
        let (graph, _index) = Self::load_with_index(path)?;
        Ok(graph)
    }

    /// ファイルからグラフとプロパティインデックスを読み込み
    pub fn load_with_index(path: impl AsRef<Path>) -> Result<(Graph, PropertyIndex)> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // マジックナンバーを確認
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(PersistenceError::InvalidMagic);
        }

        // バージョンを確認
        let version = Self::read_u32(&mut reader)?;

        if version == VERSION {
            return Self::load_v4(reader);
        }
        if version == VERSION_V5 || version == VERSION_V6 {
            // The schema is for the server; this API only carries the
            // property index definitions stored in the v4 part.
            let (snapshot, _schema) = Self::read_snapshot_v5(&mut reader)?;
            return Self::graph_from_snapshot(snapshot);
        }

        if version != VERSION_V3 && version != VERSION_V2 && version != VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion(version));
        }

        // Legacy v1/v2/v3 loading
        // ノード数とエッジ数
        let node_count = Self::read_u64(&mut reader)?;
        let edge_count = Self::read_u64(&mut reader)?;

        let mut graph = Graph::new();
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        // ノードを読み込み
        for _ in 0..node_count {
            let old_id = Self::read_u64(&mut reader)?;
            // v3: ラベルを個別に読み込み。v1/v2: コロン区切り文字列を分割
            let labels: Vec<String> = if version >= VERSION_V3 {
                Self::read_label_list(&mut reader)?
            } else {
                let labels_str = Self::read_string(&mut reader)?;
                if labels_str.is_empty() {
                    vec![]
                } else {
                    labels_str.split(':').map(|s| s.to_string()).collect()
                }
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

        // エッジを読み込み
        for _ in 0..edge_count {
            let _old_id = Self::read_u64(&mut reader)?;
            let old_from = Self::read_u64(&mut reader)?;
            let old_to = Self::read_u64(&mut reader)?;
            let label = Self::read_string(&mut reader)?;
            let properties = Self::read_properties(&mut reader)?;

            let from = *id_map.get(&old_from).ok_or_else(|| {
                PersistenceError::CorruptedData(format!("unknown node id: {}", old_from))
            })?;
            let to = *id_map.get(&old_to).ok_or_else(|| {
                PersistenceError::CorruptedData(format!("unknown node id: {}", old_to))
            })?;

            let edge_id = graph.create_edge(from, to, &label)?;

            if let Some(edge) = graph.get_edge_mut(edge_id) {
                for (key, value) in properties {
                    edge.set_property(key, value);
                }
            }
        }

        // インデックスセクションを読み込み（v2 以降のみ）
        let index = if version >= VERSION_V2 {
            Self::read_index_section(&mut reader)?
        } else {
            PropertyIndex::new()
        };

        Ok((graph, index))
    }

    /// v4 フォーマットからグラフとインデックスを読み込む
    /// Read the length-prefixed v4 payload that follows the header.
    fn read_snapshot<R: Read>(reader: &mut R) -> Result<SSnapshot> {
        let encoded = Self::read_payload(reader)?;
        snapshot_codec::decode(&encoded).map_err(PersistenceError::CorruptedData)
    }

    fn read_snapshot_v5<R: Read>(reader: &mut R) -> Result<(SSnapshot, String)> {
        let encoded = Self::read_payload(reader)?;
        snapshot_codec::decode_v5(&encoded).map_err(PersistenceError::CorruptedData)
    }

    fn read_payload<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
        let mut len_bytes = [0u8; 8];
        reader.read_exact(&mut len_bytes)?;
        let payload_len = u64::from_le_bytes(len_bytes);

        // Read through `take` rather than pre-allocating `payload_len` bytes:
        // a corrupted length then fails as a short read, not a huge allocation.
        let mut encoded = Vec::new();
        reader.take(payload_len).read_to_end(&mut encoded)?;
        if encoded.len() as u64 != payload_len {
            return Err(PersistenceError::CorruptedData(format!(
                "payload is {} byte(s) but the header says {}",
                encoded.len(),
                payload_len
            )));
        }
        Ok(encoded)
    }

    fn load_v4<R: Read>(mut reader: R) -> Result<(Graph, PropertyIndex)> {
        let snapshot = Self::read_snapshot(&mut reader)?;
        Self::graph_from_snapshot(snapshot)
    }

    fn graph_from_snapshot(snapshot: SSnapshot) -> Result<(Graph, PropertyIndex)> {
        let mut graph = Graph::new();
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        for snode in snapshot.nodes {
            let new_id = graph.create_node_with_labels(snode.labels);
            id_map.insert(snode.id, new_id);
            if let Some(node) = graph.get_node_mut(new_id) {
                for (k, v) in snode.properties {
                    node.set_property(k, v);
                }
            }
        }

        for sedge in snapshot.edges {
            let from = *id_map.get(&sedge.from).ok_or_else(|| {
                PersistenceError::CorruptedData(format!("unknown node id: {}", sedge.from))
            })?;
            let to = *id_map.get(&sedge.to).ok_or_else(|| {
                PersistenceError::CorruptedData(format!("unknown node id: {}", sedge.to))
            })?;
            let edge_id = graph.create_edge(from, to, &sedge.label)?;
            if let Some(edge) = graph.get_edge_mut(edge_id) {
                for (k, v) in sedge.properties {
                    edge.set_property(k, v);
                }
            }
        }

        let mut index = PropertyIndex::new();
        for si in snapshot.indexes {
            let def = if si.unique {
                IndexDefinition::unique(si.label, si.property)
            } else {
                IndexDefinition::new(si.label, si.property)
            };
            index.create_index(def);
        }

        Ok((graph, index))
    }

    // ========== Index section (used for legacy v1/v2/v3 reading) ==========

    fn read_index_section<R: Read>(reader: &mut R) -> Result<PropertyIndex> {
        let count = Self::read_u32(reader)? as usize;
        let mut index = PropertyIndex::new();

        for _ in 0..count {
            let label = Self::read_string(reader)?;
            let property = Self::read_string(reader)?;
            let mut unique_flag = [0u8; 1];
            reader.read_exact(&mut unique_flag)?;
            let unique = unique_flag[0] != 0;

            let def = if unique {
                IndexDefinition::unique(label, property)
            } else {
                IndexDefinition::new(label, property)
            };
            index.create_index(def);
        }

        Ok(index)
    }

    // ========== Reader helpers (used for legacy v1/v2/v3 loading) ==========

    fn read_label_list<R: Read>(reader: &mut R) -> Result<Vec<String>> {
        let count = Self::read_u32(reader)? as usize;
        let mut labels = Vec::with_capacity(count);
        for _ in 0..count {
            labels.push(Self::read_string(reader)?);
        }
        Ok(labels)
    }

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
            .map_err(|_| PersistenceError::CorruptedData("invalid UTF-8 string".to_string()))
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
                Ok(PropertyValue::Duration {
                    months,
                    days,
                    millis,
                })
            }
            t => Err(PersistenceError::CorruptedData(format!(
                "unknown property type: {}",
                t
            ))),
        }
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // 3.14 等は PI の近似ではなくリテラルのテストデータ
mod tests {
    use super::*;

    // ── v4 files written by bincode 1 (before snapshot_codec) ──────────
    // Generated on 2026-11-03 with the bincode-based writer: nodes 0..7 have
    // labels [Value, <TYPE>] and one property of each PropertyValue variant,
    // node 8 has no labels; edges NEXT(0->1, w: 0.5) and TO_BARE(2->8).
    // `v4_one_index` has a unique index on Value.string, `v4_two_indexes`
    // adds a non-unique one on Value.int.

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn expected_values() -> Vec<(&'static str, PropertyValue)> {
        vec![
            ("null", PropertyValue::Null),
            ("bool", PropertyValue::Bool(true)),
            ("int", PropertyValue::Int(-42)),
            ("float", PropertyValue::Float(-1.25e-3)),
            (
                "string",
                PropertyValue::String("日本語 & \"quotes\"".to_string()),
            ),
            ("date", PropertyValue::Date(-719_162)),
            ("datetime", PropertyValue::DateTime(1_700_000_000_123)),
            (
                "duration",
                PropertyValue::Duration {
                    months: 14,
                    days: -3,
                    millis: 86_400_001,
                },
            ),
        ]
    }

    #[test]
    fn test_load_bincode_written_v4_fixture() {
        let (graph, index) =
            PersistentStorage::load_with_index(fixture("v4_two_indexes.mgdb")).unwrap();
        assert_eq!(graph.node_count(), 9);
        assert_eq!(graph.edge_count(), 2);
        for (i, (key, value)) in expected_values().into_iter().enumerate() {
            let node = graph.get_node(i as u64).unwrap();
            assert_eq!(node.labels, vec!["Value".to_string(), key.to_uppercase()]);
            assert_eq!(node.get_property(key), Some(&value), "{key}");
        }
        assert!(graph.get_node(8).unwrap().labels.is_empty());

        let mut edges: Vec<_> = graph.edges().collect();
        edges.sort_by_key(|e| e.id);
        assert_eq!(
            (edges[0].label.as_str(), edges[0].from, edges[0].to),
            ("NEXT", 0, 1)
        );
        assert_eq!(edges[0].get_property("w"), Some(&PropertyValue::Float(0.5)));
        assert_eq!(
            (edges[1].label.as_str(), edges[1].from, edges[1].to),
            ("TO_BARE", 2, 8)
        );

        assert!(index.has_index("Value", "string"));
        assert!(index.has_index("Value", "int"));
        let unique: Vec<_> = index
            .list_indexes()
            .into_iter()
            .map(|d| (d.property.as_str(), d.unique))
            .collect();
        assert!(unique.contains(&("string", true)));
        assert!(unique.contains(&("int", false)));

        let concurrent =
            PersistentStorage::load_concurrent(fixture("v4_two_indexes.mgdb")).unwrap();
        assert_eq!(concurrent.node_count(), 9);
        assert_eq!(concurrent.edge_count(), 2);
    }

    /// Writing the loaded graph again produces the bincode-written bytes exactly.
    #[test]
    fn test_v4_writer_matches_bincode_bytes() {
        let original = std::fs::read(fixture("v4_one_index.mgdb")).unwrap();
        let (graph, index) =
            PersistentStorage::load_with_index(fixture("v4_one_index.mgdb")).unwrap();
        let path =
            std::env::temp_dir().join(format!("maharit_v4_rewrite_{}.mgdb", std::process::id()));
        PersistentStorage::save_with_index(&graph, &index, &path).unwrap();
        let rewritten = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(rewritten, original);
    }

    /// Corrupted v4 files are rejected with an error, never a panic or a
    /// huge allocation.
    #[test]
    fn test_corrupted_v4_files_are_rejected() {
        let original = std::fs::read(fixture("v4_one_index.mgdb")).unwrap();
        let header = 8 + 4 + 8; // magic, version, payload length
        let path =
            std::env::temp_dir().join(format!("maharit_v4_corrupt_{}.mgdb", std::process::id()));
        let load = |bytes: &[u8]| {
            std::fs::write(&path, bytes).unwrap();
            PersistentStorage::load_with_index(&path).map(|_| ())
        };

        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        // Payload length far larger than the file.
        let mut huge_len = original.clone();
        huge_len[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
        cases.push(("huge payload length", huge_len));
        // Node count far larger than the payload.
        let mut huge_count = original.clone();
        huge_count[header..header + 8].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
        cases.push(("huge node count", huge_count));
        // Truncated payload (header length still claims the full size).
        cases.push(("truncated", original[..original.len() - 10].to_vec()));
        // Extra bytes inside the declared payload.
        let mut trailing = original.clone();
        trailing.push(0);
        let len = (original.len() - header + 1) as u64;
        trailing[12..20].copy_from_slice(&len.to_le_bytes());
        cases.push(("trailing bytes", trailing));
        // First property's type tag of node 0 replaced with an unknown variant.
        // Layout: count(8) id(8) labels: count(8) "Value"(8+5) "NULL"(8+4) props count(8) key "null"(8+4) tag
        let tag_at = header + 8 + 8 + 8 + 13 + 12 + 8 + 12;
        let mut bad_tag = original.clone();
        assert_eq!(
            &bad_tag[tag_at..tag_at + 4],
            &0u32.to_le_bytes(),
            "layout drifted"
        );
        bad_tag[tag_at..tag_at + 4].copy_from_slice(&99u32.to_le_bytes());
        cases.push(("unknown property type", bad_tag));

        for (name, bytes) in cases {
            assert!(
                matches!(
                    load(&bytes),
                    Err(PersistenceError::CorruptedData(_)) | Err(PersistenceError::Io(_))
                ),
                "{name} should be rejected"
            );
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_save_load_empty_graph() {
        let graph = Graph::new();
        let path = "/tmp/maharit_test_empty.db";

        PersistentStorage::save(&graph, path).unwrap();
        let loaded = PersistentStorage::load(path).unwrap();

        assert_eq!(loaded.node_count(), 0);
        assert_eq!(loaded.edge_count(), 0);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_load_nodes() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
            node.set_property("active", true);
        }

        let path = "/tmp/maharit_test_nodes.db";

        PersistentStorage::save(&graph, path).unwrap();
        let loaded = PersistentStorage::load(path).unwrap();

        assert_eq!(loaded.node_count(), 1);

        let node = loaded.nodes().next().unwrap();
        assert!(node.has_label("Person"));
        assert_eq!(
            node.properties.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(node.properties.get("age"), Some(&PropertyValue::Int(30)));
        assert_eq!(
            node.properties.get("active"),
            Some(&PropertyValue::Bool(true))
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_load_with_edges() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(a) {
            node.set_property("name", "Alice");
        }
        if let Some(node) = graph.get_node_mut(b) {
            node.set_property("name", "Bob");
        }

        let edge_id = graph.create_edge(a, b, "KNOWS").unwrap();
        if let Some(edge) = graph.get_edge_mut(edge_id) {
            edge.set_property("since", 2020);
        }

        let path = "/tmp/maharit_test_edges.db";

        PersistentStorage::save(&graph, path).unwrap();
        let loaded = PersistentStorage::load(path).unwrap();

        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);

        let edge = loaded.edges().next().unwrap();
        assert_eq!(edge.label, "KNOWS");
        assert_eq!(
            edge.properties.get("since"),
            Some(&PropertyValue::Int(2020))
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_invalid_magic() {
        let path = "/tmp/maharit_test_invalid.db";
        std::fs::write(path, b"INVALID!").unwrap();

        let result = PersistentStorage::load(path);
        assert!(matches!(result, Err(PersistenceError::InvalidMagic)));

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_property_types() {
        let mut graph = Graph::new();
        let id = graph.create_node("Test");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("null_val", PropertyValue::Null);
            node.set_property("bool_val", true);
            node.set_property("int_val", 42i64);
            node.set_property("float_val", 3.14f64);
            node.set_property("string_val", "hello");
        }

        let path = "/tmp/maharit_test_types.db";

        PersistentStorage::save(&graph, path).unwrap();
        let loaded = PersistentStorage::load(path).unwrap();

        let node = loaded.nodes().next().unwrap();
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
            Some(&PropertyValue::Float(3.14))
        );
        assert_eq!(
            node.properties.get("string_val"),
            Some(&PropertyValue::String("hello".to_string()))
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_load_with_index() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(alice) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }

        let mut index = PropertyIndex::new();
        index.create_index(IndexDefinition::new("Person", "name"));
        index.create_index(IndexDefinition::unique("Person", "email"));

        let path = "/tmp/maharit_test_index.db";

        PersistentStorage::save_with_index(&graph, &index, path).unwrap();
        let (loaded_graph, loaded_index) = PersistentStorage::load_with_index(path).unwrap();

        assert_eq!(loaded_graph.node_count(), 1);
        assert!(loaded_index.has_index("Person", "name"));
        assert!(loaded_index.has_index("Person", "email"));
        assert!(!loaded_index.has_index("Person", "age"));

        // unique フラグの確認
        let defs = loaded_index.list_indexes();
        let email_def = defs.iter().find(|d| d.property == "email").unwrap();
        assert!(email_def.unique);
        let name_def = defs.iter().find(|d| d.property == "name").unwrap();
        assert!(!name_def.unique);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_load_multiple_indexes() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        graph.create_node("Company");

        let mut index = PropertyIndex::new();
        index.create_index(IndexDefinition::new("Person", "name"));
        index.create_index(IndexDefinition::new("Person", "age"));
        index.create_index(IndexDefinition::new("Company", "name"));
        index.create_index(IndexDefinition::unique("Company", "ticker"));

        let path = "/tmp/maharit_test_multi_index.db";

        PersistentStorage::save_with_index(&graph, &index, path).unwrap();
        let (loaded_graph, loaded_index) = PersistentStorage::load_with_index(path).unwrap();

        assert_eq!(loaded_graph.node_count(), 2);
        assert!(loaded_index.has_index("Person", "name"));
        assert!(loaded_index.has_index("Person", "age"));
        assert!(loaded_index.has_index("Company", "name"));
        assert!(loaded_index.has_index("Company", "ticker"));
        assert!(!loaded_index.has_index("Person", "ticker"));

        let defs = loaded_index.list_indexes();
        assert_eq!(defs.len(), 4);

        let ticker_def = defs.iter().find(|d| d.property == "ticker").unwrap();
        assert!(ticker_def.unique);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_load_multi_label_nodes() {
        let mut graph = Graph::new();
        let id = graph.create_node_with_labels(vec!["Person".to_string(), "Employee".to_string()]);
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", "Alice");
        }

        let path = "/tmp/maharit_test_multi_label.db";

        PersistentStorage::save(&graph, path).unwrap();
        let loaded = PersistentStorage::load(path).unwrap();

        assert_eq!(loaded.node_count(), 1);
        let node = loaded.nodes().next().unwrap();
        assert!(node.has_label("Person"));
        assert!(node.has_label("Employee"));
        assert_eq!(node.labels.len(), 2);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_save_without_index_load_with_index() {
        // save() を使って保存した場合も load_with_index() で読める（空インデックスが返る）
        let mut graph = Graph::new();
        graph.create_node("Person");

        let path = "/tmp/maharit_test_no_index.db";
        PersistentStorage::save(&graph, path).unwrap();

        let (loaded_graph, loaded_index) = PersistentStorage::load_with_index(path).unwrap();
        assert_eq!(loaded_graph.node_count(), 1);
        assert_eq!(loaded_index.list_indexes().len(), 0);

        std::fs::remove_file(path).ok();
    }

    // ── save_concurrent / load_concurrent ラウンドトリップ ───────────────────

    #[test]
    fn test_concurrent_roundtrip_nodes_and_properties() {
        use maharit_core::ConcurrentGraph;

        let graph = ConcurrentGraph::new();
        let id = graph.create_node_with_labels(vec!["Person".to_string()]);
        graph.set_node_property(id, "name", PropertyValue::String("Alice".to_string()));
        graph.set_node_property(id, "age", PropertyValue::Int(30));
        graph.set_node_property(id, "active", PropertyValue::Bool(true));
        graph.set_node_property(id, "score", PropertyValue::Float(9.5));

        let path = "/tmp/maharit_test_concurrent_props.db";
        PersistentStorage::save_concurrent(&graph, path).unwrap();

        let loaded = PersistentStorage::load_concurrent(path).unwrap();
        assert_eq!(loaded.node_count(), 1);

        let name = loaded
            .with_node(id, |n| n.properties.get("name").cloned())
            .unwrap();
        let age = loaded
            .with_node(id, |n| n.properties.get("age").cloned())
            .unwrap();
        let active = loaded
            .with_node(id, |n| n.properties.get("active").cloned())
            .unwrap();
        let score = loaded
            .with_node(id, |n| n.properties.get("score").cloned())
            .unwrap();

        assert_eq!(name, Some(PropertyValue::String("Alice".to_string())));
        assert_eq!(age, Some(PropertyValue::Int(30)));
        assert_eq!(active, Some(PropertyValue::Bool(true)));
        if let Some(PropertyValue::Float(f)) = score {
            assert!((f - 9.5).abs() < 1e-6);
        } else {
            panic!("expected Float, got {:?}", score);
        }

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_list_properties_use_v6_and_round_trip() {
        // graph/146: リストがあれば v6、無ければ従来どおり v4 / v5 で書く
        use maharit_core::ConcurrentGraph;
        let version_of = |path: &std::path::Path| {
            let bytes = std::fs::read(path).unwrap();
            u32::from_le_bytes(bytes[8..12].try_into().unwrap())
        };
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let list = PropertyValue::List(vec![
            PropertyValue::String("x".into()),
            PropertyValue::Int(2),
            PropertyValue::Float(1.0),
            PropertyValue::Null,
        ]);

        let mut graph = Graph::new();
        let id = graph.create_node("P");
        let plain = dir.join(format!("maharit_v6_plain_{pid}.mgdb"));
        PersistentStorage::save(&graph, &plain).unwrap();
        assert_eq!(version_of(&plain), VERSION);

        graph
            .get_node_mut(id)
            .unwrap()
            .set_property("tags", list.clone());
        let with_list = dir.join(format!("maharit_v6_list_{pid}.mgdb"));
        PersistentStorage::save(&graph, &with_list).unwrap();
        assert_eq!(version_of(&with_list), VERSION_V6);
        let loaded = PersistentStorage::load(&with_list).unwrap();
        assert_eq!(
            loaded.get_node(id).unwrap().get_property("tags"),
            Some(&list)
        );

        let cg = ConcurrentGraph::new();
        let cid = cg.create_node_with_labels(vec!["P".to_string()]);
        let concurrent = dir.join(format!("maharit_v6_concurrent_{pid}.mgdb"));
        PersistentStorage::save_concurrent_with_schema(&cg, "s", &concurrent).unwrap();
        assert_eq!(version_of(&concurrent), VERSION_V5);
        cg.set_node_property(cid, "tags", list.clone());
        PersistentStorage::save_concurrent_with_schema(&cg, "s", &concurrent).unwrap();
        assert_eq!(version_of(&concurrent), VERSION_V6);
        let (loaded, schema) = PersistentStorage::load_concurrent_with_schema(&concurrent).unwrap();
        assert_eq!(schema.as_deref(), Some("s"));
        assert_eq!(
            loaded
                .with_node(cid, |n| n.properties.get("tags").cloned())
                .unwrap(),
            Some(list)
        );
        let (_, schema) = PersistentStorage::load_with_schema(&concurrent).unwrap();
        assert_eq!(schema.as_deref(), Some("s"));

        for p in [plain, with_list, concurrent] {
            std::fs::remove_file(p).ok();
        }
    }

    #[test]
    fn test_concurrent_roundtrip_edges() {
        use maharit_core::ConcurrentGraph;

        let graph = ConcurrentGraph::new();
        let a = graph.create_node_with_labels(vec!["Src".to_string()]);
        let b = graph.create_node_with_labels(vec!["Dst".to_string()]);
        graph.create_edge(a, b, "LINK").unwrap();

        let path = "/tmp/maharit_test_concurrent_edges.db";
        PersistentStorage::save_concurrent(&graph, path).unwrap();

        let loaded = PersistentStorage::load_concurrent(path).unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);

        let edge = loaded.edges().next().unwrap();
        let edge = edge.value();
        assert_eq!(edge.label, "LINK");
        let src_labels = loaded.with_node(edge.from, |n| n.labels.clone()).unwrap();
        let dst_labels = loaded.with_node(edge.to, |n| n.labels.clone()).unwrap();
        assert!(
            src_labels.contains(&"Src".to_string()) || dst_labels.contains(&"Src".to_string()),
            "エッジの端点に Src ラベルが存在すること"
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_concurrent_roundtrip_preserves_ids() {
        use maharit_core::ConcurrentGraph;

        // Sparse IDs (deleted nodes/edges leave gaps) must survive a reload:
        // the server WAL written after a checkpoint addresses them by ID.
        let graph = ConcurrentGraph::new();
        let ids: Vec<u64> = (0..6)
            .map(|i| graph.create_node_with_labels(vec![format!("L{i}")]))
            .collect();
        let e0 = graph.create_edge(ids[0], ids[5], "A").unwrap();
        let e1 = graph.create_edge(ids[3], ids[5], "B").unwrap();
        graph.delete_edge(e0);
        graph.delete_node(ids[1]);
        graph.delete_node(ids[2]);

        let path = std::env::temp_dir().join(format!(
            "maharit_test_concurrent_ids_{}.db",
            std::process::id()
        ));
        PersistentStorage::save_concurrent(&graph, &path).unwrap();
        let loaded = PersistentStorage::load_concurrent(&path).unwrap();

        for &id in &[ids[0], ids[3], ids[4], ids[5]] {
            assert_eq!(
                loaded.with_node(id, |n| n.labels.clone()),
                graph.with_node(id, |n| n.labels.clone()),
                "node {id}"
            );
        }
        assert!(!loaded.contains_node(ids[1]));
        let edge = loaded.edges().next().unwrap();
        assert_eq!(*edge.key(), e1);
        assert_eq!((edge.value().from, edge.value().to), (ids[3], ids[5]));
        // New IDs never collide with loaded ones.
        let fresh = loaded.create_node_with_labels(vec![]);
        assert!(fresh > ids[5]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_concurrent_roundtrip_labels() {
        use maharit_core::ConcurrentGraph;

        let graph = ConcurrentGraph::new();
        let id = graph.create_node_with_labels(vec!["Person".to_string(), "Employee".to_string()]);

        let path = "/tmp/maharit_test_concurrent_labels.db";
        PersistentStorage::save_concurrent(&graph, path).unwrap();

        let loaded = PersistentStorage::load_concurrent(path).unwrap();
        let labels = loaded.with_node(id, |n| n.labels.clone()).unwrap();
        assert!(labels.contains(&"Person".to_string()));
        assert!(labels.contains(&"Employee".to_string()));

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_concurrent_roundtrip_large_dataset() {
        use maharit_core::ConcurrentGraph;

        let graph = ConcurrentGraph::new();

        // 100件のノードを作成
        let mut ids = Vec::new();
        for i in 0..100u64 {
            let id = graph.create_node_with_labels(vec!["Item".to_string()]);
            graph.set_node_property(id, "idx", PropertyValue::Int(i as i64));
            graph.set_node_property(id, "name", PropertyValue::String(format!("item-{}", i)));
            ids.push(id);
        }

        // 50件のエッジを作成
        for i in 0..50usize {
            graph.create_edge(ids[i], ids[i + 1], "NEXT").ok();
        }

        let path = "/tmp/maharit_test_concurrent_large.db";
        PersistentStorage::save_concurrent(&graph, path).unwrap();

        let loaded = PersistentStorage::load_concurrent(path).unwrap();
        assert_eq!(loaded.node_count(), 100, "全ノードが復元されること");
        assert_eq!(loaded.edge_count(), 50, "全エッジが復元されること");

        // load_concurrent はノードIDをリマップするため、
        // プロパティ値でノードを探して検証する
        let found_42 = loaded.nodes().any(|r| {
            let n = r.value();
            n.properties.get("idx") == Some(&PropertyValue::Int(42))
        });
        assert!(found_42, "idx=42 のノードが復元されていること");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_concurrent_roundtrip_empty_graph() {
        use maharit_core::ConcurrentGraph;

        let graph = ConcurrentGraph::new();
        let path = "/tmp/maharit_test_concurrent_empty.db";
        PersistentStorage::save_concurrent(&graph, path).unwrap();

        let loaded = PersistentStorage::load_concurrent(path).unwrap();
        assert_eq!(loaded.node_count(), 0);
        assert_eq!(loaded.edge_count(), 0);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn v5_round_trips_the_schema_and_stays_readable_by_every_loader() {
        let path = std::env::temp_dir().join(format!("maharit_v5_{}.mgdb", std::process::id()));
        let graph = ConcurrentGraph::new();
        let a = graph.create_node("Person");
        graph.set_node_property(a, "name", PropertyValue::String("Alice".into()));
        let b = graph.create_node("Person");
        graph.create_edge(a, b, "KNOWS").unwrap();

        let schema = r#"{"constraints":[{"name":"u"}]}"#;
        PersistentStorage::save_concurrent_with_schema(&graph, schema, &path).unwrap();
        let (loaded, got) = PersistentStorage::load_concurrent_with_schema(&path).unwrap();
        assert_eq!(got.as_deref(), Some(schema));
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
        // The Graph loader reads v5 too (ignoring the schema).
        let g = PersistentStorage::load(&path).unwrap();
        assert_eq!(g.node_count(), 2);

        // An empty schema reads back as none.
        PersistentStorage::save_concurrent(&graph, &path).unwrap();
        let (_, got) = PersistentStorage::load_concurrent_with_schema(&path).unwrap();
        assert_eq!(got, None);

        // A truncated v5 file is rejected.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        assert!(PersistentStorage::load_concurrent_with_schema(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn v4_files_load_without_a_schema() {
        let (graph, schema) =
            PersistentStorage::load_concurrent_with_schema(fixture("v4_two_indexes.mgdb")).unwrap();
        assert!(graph.node_count() > 0);
        assert_eq!(schema, None);
    }
}
