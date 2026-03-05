use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use maharit_core::{Graph, IndexDefinition, PropertyIndex, PropertyValue};
use thiserror::Error;

/// ファイルフォーマットのマジックナンバー
const MAGIC: &[u8; 8] = b"MAHARITD";
/// フォーマットバージョン（v3: ラベルを Vec<String> として保存）
const VERSION: u32 = 3;
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

    /// グラフとプロパティインデックスをファイルに保存
    pub fn save_with_index(
        graph: &Graph,
        index: &PropertyIndex,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::with_capacity(BUF_CAPACITY, file);

        // ヘッダー
        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;

        // ノード数とエッジ数
        let node_count = graph.node_count() as u64;
        let edge_count = graph.edge_count() as u64;
        writer.write_all(&node_count.to_le_bytes())?;
        writer.write_all(&edge_count.to_le_bytes())?;

        // ノードを書き込み
        for node in graph.nodes() {
            Self::write_u64(&mut writer, node.id)?;
            // v3: ラベルを個別に保存（count + each string）
            Self::write_label_list(&mut writer, &node.labels)?;
            Self::write_properties(&mut writer, &node.properties)?;
        }

        // エッジを書き込み
        for edge in graph.edges() {
            Self::write_u64(&mut writer, edge.id)?;
            Self::write_u64(&mut writer, edge.from)?;
            Self::write_u64(&mut writer, edge.to)?;
            Self::write_string(&mut writer, &edge.label)?;
            Self::write_properties(&mut writer, &edge.properties)?;
        }

        // インデックスセクションを書き込み
        Self::write_index_section(&mut writer, index)?;

        writer.flush()?;
        Ok(())
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
        if version != VERSION && version != VERSION_V2 && version != VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion(version));
        }

        // ノード数とエッジ数
        let node_count = Self::read_u64(&mut reader)?;
        let edge_count = Self::read_u64(&mut reader)?;

        let mut graph = Graph::new();
        let mut id_map: HashMap<u64, u64> = HashMap::new();

        // ノードを読み込み
        for _ in 0..node_count {
            let old_id = Self::read_u64(&mut reader)?;
            // v3: ラベルを個別に読み込み。v1/v2: コロン区切り文字列を分割
            let labels: Vec<String> = if version >= VERSION {
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

    // ========== Index section ==========

    fn write_index_section<W: Write>(writer: &mut W, index: &PropertyIndex) -> Result<()> {
        let definitions = index.list_indexes();
        let count = definitions.len() as u32;
        writer.write_all(&count.to_le_bytes())?;

        for def in definitions {
            Self::write_string(writer, &def.label)?;
            Self::write_string(writer, &def.property)?;
            // unique フラグを保存
            writer.write_all(&[if def.unique { 1u8 } else { 0u8 }])?;
        }

        Ok(())
    }

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

    // ========== Writer helpers ==========

    fn write_u64<W: Write>(writer: &mut W, value: u64) -> Result<()> {
        writer.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    fn write_label_list<W: Write>(writer: &mut W, labels: &[String]) -> Result<()> {
        writer.write_all(&(labels.len() as u32).to_le_bytes())?;
        for label in labels {
            Self::write_string(writer, label)?;
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
            t => Err(PersistenceError::CorruptedData(format!(
                "unknown property type: {}",
                t
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
