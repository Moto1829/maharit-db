use std::collections::HashMap;
use std::io::{Read, Write};

use maharit_core::{Graph, PropertyValue};

use crate::{IoError, Result};

/// CSVインポーター
pub struct CsvImporter;

impl CsvImporter {
    /// ノードCSVをインポート
    /// フォーマット: id,label,prop1,prop2,...
    /// 最初の行はヘッダー
    pub fn import_nodes<R: Read>(graph: &mut Graph, reader: R) -> Result<ImportStats> {
        let mut csv_reader = csv::Reader::from_reader(reader);
        let headers: Vec<String> = csv_reader
            .headers()?
            .iter()
            .map(|s| s.to_string())
            .collect();

        if headers.len() < 2 {
            return Err(IoError::InvalidFormat(
                "Node CSV must have at least 'id' and 'label' columns".to_string(),
            ));
        }

        let mut id_map: HashMap<String, u64> = HashMap::new();
        let mut count = 0;

        for result in csv_reader.records() {
            let record = result?;

            // id列（外部ID）
            let external_id = record.get(0).unwrap_or("").to_string();

            // label列（コロン区切りで複数ラベルを格納）
            let label_str = record.get(1).unwrap_or("").to_string();
            let labels: Vec<String> = if label_str.is_empty() {
                vec![]
            } else {
                label_str.split(':').map(|s| s.to_string()).collect()
            };

            // ノードを作成
            let node_id = graph.create_node_with_labels(labels);
            id_map.insert(external_id, node_id);

            // プロパティを設定
            if let Some(node) = graph.get_node_mut(node_id) {
                for (i, header) in headers.iter().enumerate().skip(2) {
                    if let Some(value) = record.get(i)
                        && !value.is_empty()
                    {
                        let prop_value = Self::parse_value(value);
                        node.set_property(header.clone(), prop_value);
                    }
                }
            }

            count += 1;
        }

        Ok(ImportStats {
            nodes_imported: count,
            edges_imported: 0,
            id_map,
        })
    }

    /// エッジCSVをインポート
    /// フォーマット: from,to,type,prop1,prop2,...
    /// from/toは外部ID（import_nodesで返されたid_mapを使用）
    pub fn import_edges<R: Read>(
        graph: &mut Graph,
        reader: R,
        id_map: &HashMap<String, u64>,
    ) -> Result<ImportStats> {
        let mut csv_reader = csv::Reader::from_reader(reader);
        let headers: Vec<String> = csv_reader
            .headers()?
            .iter()
            .map(|s| s.to_string())
            .collect();

        if headers.len() < 3 {
            return Err(IoError::InvalidFormat(
                "Edge CSV must have at least 'from', 'to', and 'type' columns".to_string(),
            ));
        }

        let mut count = 0;

        for result in csv_reader.records() {
            let record = result?;

            let from_external = record.get(0).unwrap_or("");
            let to_external = record.get(1).unwrap_or("");
            let edge_type = record.get(2).unwrap_or("").to_string();

            // 外部IDから内部IDに変換
            let from_id = id_map.get(from_external).copied().ok_or_else(|| {
                IoError::InvalidFormat(format!("Unknown node ID: {}", from_external))
            })?;

            let to_id = id_map.get(to_external).copied().ok_or_else(|| {
                IoError::InvalidFormat(format!("Unknown node ID: {}", to_external))
            })?;

            // エッジを作成
            let edge_id = graph.create_edge(from_id, to_id, &edge_type)?;

            // プロパティを設定
            if let Some(edge) = graph.get_edge_mut(edge_id) {
                for (i, header) in headers.iter().enumerate().skip(3) {
                    if let Some(value) = record.get(i)
                        && !value.is_empty()
                    {
                        let prop_value = Self::parse_value(value);
                        edge.set_property(header.clone(), prop_value);
                    }
                }
            }

            count += 1;
        }

        Ok(ImportStats {
            nodes_imported: 0,
            edges_imported: count,
            id_map: HashMap::new(),
        })
    }

    /// 文字列から適切な型に変換
    fn parse_value(s: &str) -> PropertyValue {
        // 整数を試す
        if let Ok(n) = s.parse::<i64>() {
            return PropertyValue::Int(n);
        }

        // 浮動小数点を試す
        if let Ok(n) = s.parse::<f64>() {
            return PropertyValue::Float(n);
        }

        // 真偽値を試す
        match s.to_lowercase().as_str() {
            "true" => return PropertyValue::Bool(true),
            "false" => return PropertyValue::Bool(false),
            "null" | "" => return PropertyValue::Null,
            _ => {}
        }

        // 文字列として扱う
        PropertyValue::String(s.to_string())
    }
}

/// インポート統計
#[derive(Debug, Clone)]
pub struct ImportStats {
    pub nodes_imported: usize,
    pub edges_imported: usize,
    pub id_map: HashMap<String, u64>,
}

/// CSVエクスポーター
pub struct CsvExporter;

impl CsvExporter {
    /// ノードをCSVにエクスポート
    pub fn export_nodes<W: Write>(graph: &Graph, writer: W) -> Result<usize> {
        let mut csv_writer = csv::Writer::from_writer(writer);

        // 全プロパティキーを収集
        let mut all_keys: Vec<String> = Vec::new();
        for node in graph.nodes() {
            for key in node.properties.keys() {
                if !all_keys.contains(key) {
                    all_keys.push(key.clone());
                }
            }
        }
        all_keys.sort();

        // ヘッダーを書き込み
        let mut headers = vec!["id".to_string(), "label".to_string()];
        headers.extend(all_keys.clone());
        csv_writer.write_record(&headers)?;

        // ノードを書き込み
        let mut count = 0;
        for node in graph.nodes() {
            let mut record = vec![node.id.to_string(), node.labels.join(":")];

            for key in &all_keys {
                let value = node
                    .properties
                    .get(key)
                    .map(Self::format_value)
                    .unwrap_or_default();
                record.push(value);
            }

            csv_writer.write_record(&record)?;
            count += 1;
        }

        csv_writer.flush()?;
        Ok(count)
    }

    /// エッジをCSVにエクスポート
    pub fn export_edges<W: Write>(graph: &Graph, writer: W) -> Result<usize> {
        let mut csv_writer = csv::Writer::from_writer(writer);

        // 全プロパティキーを収集
        let mut all_keys: Vec<String> = Vec::new();
        for edge in graph.edges() {
            for key in edge.properties.keys() {
                if !all_keys.contains(key) {
                    all_keys.push(key.clone());
                }
            }
        }
        all_keys.sort();

        // ヘッダーを書き込み
        let mut headers = vec!["from".to_string(), "to".to_string(), "type".to_string()];
        headers.extend(all_keys.clone());
        csv_writer.write_record(&headers)?;

        // エッジを書き込み
        let mut count = 0;
        for edge in graph.edges() {
            let mut record = vec![
                edge.from.to_string(),
                edge.to.to_string(),
                edge.label.clone(),
            ];

            for key in &all_keys {
                let value = edge
                    .properties
                    .get(key)
                    .map(Self::format_value)
                    .unwrap_or_default();
                record.push(value);
            }

            csv_writer.write_record(&record)?;
            count += 1;
        }

        csv_writer.flush()?;
        Ok(count)
    }

    fn format_value(v: &PropertyValue) -> String {
        match v {
            PropertyValue::Null => String::new(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Int(n) => n.to_string(),
            PropertyValue::Float(n) => n.to_string(),
            PropertyValue::String(s) => s.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_nodes() {
        let csv_data = "id,label,name,age\n1,Person,Alice,30\n2,Person,Bob,25\n";
        let mut graph = Graph::new();

        let stats = CsvImporter::import_nodes(&mut graph, csv_data.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(graph.node_count(), 2);

        // IDマップを確認
        assert!(stats.id_map.contains_key("1"));
        assert!(stats.id_map.contains_key("2"));
    }

    #[test]
    fn test_import_edges() {
        let nodes_csv = "id,label,name\n1,Person,Alice\n2,Person,Bob\n";
        let edges_csv = "from,to,type,since\n1,2,KNOWS,2020\n";

        let mut graph = Graph::new();
        let node_stats = CsvImporter::import_nodes(&mut graph, nodes_csv.as_bytes()).unwrap();
        let edge_stats =
            CsvImporter::import_edges(&mut graph, edges_csv.as_bytes(), &node_stats.id_map)
                .unwrap();

        assert_eq!(edge_stats.edges_imported, 1);
        assert_eq!(graph.edge_count(), 1);

        let edge = graph.edges().next().unwrap();
        assert_eq!(edge.label, "KNOWS");
        assert_eq!(
            edge.properties.get("since"),
            Some(&PropertyValue::Int(2020))
        );
    }

    #[test]
    fn test_export_nodes() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }

        let mut output = Vec::new();
        let count = CsvExporter::export_nodes(&graph, &mut output).unwrap();

        assert_eq!(count, 1);

        let csv_str = String::from_utf8(output).unwrap();
        assert!(csv_str.contains("id,label,"));
        assert!(csv_str.contains("Person"));
        assert!(csv_str.contains("Alice"));
    }

    #[test]
    fn test_export_edges() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");
        graph.create_edge(a, b, "KNOWS").unwrap();

        let mut output = Vec::new();
        let count = CsvExporter::export_edges(&graph, &mut output).unwrap();

        assert_eq!(count, 1);

        let csv_str = String::from_utf8(output).unwrap();
        assert!(csv_str.contains("from,to,type"));
        assert!(csv_str.contains("KNOWS"));
    }

    #[test]
    fn test_roundtrip() {
        // 元のグラフを作成
        let mut graph1 = Graph::new();
        let a = graph1.create_node("Person");
        let b = graph1.create_node("Person");
        if let Some(node) = graph1.get_node_mut(a) {
            node.set_property("name", "Alice");
        }
        if let Some(node) = graph1.get_node_mut(b) {
            node.set_property("name", "Bob");
        }
        graph1.create_edge(a, b, "KNOWS").unwrap();

        // エクスポート
        let mut nodes_csv = Vec::new();
        let mut edges_csv = Vec::new();
        CsvExporter::export_nodes(&graph1, &mut nodes_csv).unwrap();
        CsvExporter::export_edges(&graph1, &mut edges_csv).unwrap();

        // インポート
        let mut graph2 = Graph::new();
        let stats = CsvImporter::import_nodes(&mut graph2, nodes_csv.as_slice()).unwrap();
        CsvImporter::import_edges(&mut graph2, edges_csv.as_slice(), &stats.id_map).unwrap();

        assert_eq!(graph2.node_count(), 2);
        assert_eq!(graph2.edge_count(), 1);
    }
}
