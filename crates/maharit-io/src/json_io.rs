use std::collections::HashMap;
use std::io::{Read, Write};

use maharit_core::{Graph, PropertyValue};
use serde::{Deserialize, Serialize};

use crate::{IoError, Result};

/// ノードのJSON表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// エッジのJSON表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEdge {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// グラフ全体のJSON表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonGraph {
    pub nodes: Vec<JsonNode>,
    pub edges: Vec<JsonEdge>,
}

// ========== 隣接リスト形式 ==========

/// 隣接リスト形式でのエッジ表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjacencyEdge {
    pub to: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// 隣接リスト形式でのノード表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjacencyNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub edges: Vec<AdjacencyEdge>,
}

/// 隣接リスト形式のグラフ表現
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjacencyGraph {
    pub nodes: Vec<AdjacencyNode>,
}

/// JSONインポーター
pub struct JsonImporter;

impl JsonImporter {
    /// JSONからグラフをインポート
    pub fn import<R: Read>(graph: &mut Graph, reader: R) -> Result<ImportStats> {
        let json_graph: JsonGraph = serde_json::from_reader(reader)?;

        let mut id_map: HashMap<String, u64> = HashMap::new();

        // ノードをインポート
        for json_node in &json_graph.nodes {
            // Labels may be stored as colon-separated string for multi-label support
            let labels: Vec<String> = if json_node.label.is_empty() {
                vec![]
            } else {
                json_node
                    .label
                    .split(':')
                    .map(|s| s.to_string())
                    .collect()
            };
            let node_id = graph.create_node_with_labels(labels);
            id_map.insert(json_node.id.clone(), node_id);

            if let Some(node) = graph.get_node_mut(node_id) {
                for (key, value) in &json_node.properties {
                    node.set_property(key.clone(), Self::json_to_property(value));
                }
            }
        }

        // エッジをインポート
        let mut edges_imported = 0;
        for json_edge in &json_graph.edges {
            let from_id = id_map.get(&json_edge.from).copied().ok_or_else(|| {
                IoError::InvalidFormat(format!("Unknown node ID: {}", json_edge.from))
            })?;

            let to_id = id_map.get(&json_edge.to).copied().ok_or_else(|| {
                IoError::InvalidFormat(format!("Unknown node ID: {}", json_edge.to))
            })?;

            let edge_id = graph.create_edge(from_id, to_id, &json_edge.edge_type)?;

            if let Some(edge) = graph.get_edge_mut(edge_id) {
                for (key, value) in &json_edge.properties {
                    edge.set_property(key.clone(), Self::json_to_property(value));
                }
            }

            edges_imported += 1;
        }

        Ok(ImportStats {
            nodes_imported: json_graph.nodes.len(),
            edges_imported,
        })
    }

    fn json_to_property(value: &serde_json::Value) -> PropertyValue {
        match value {
            serde_json::Value::Null => PropertyValue::Null,
            serde_json::Value::Bool(b) => PropertyValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PropertyValue::Int(i)
                } else if let Some(f) = n.as_f64() {
                    PropertyValue::Float(f)
                } else {
                    PropertyValue::Null
                }
            }
            serde_json::Value::String(s) => PropertyValue::String(s.clone()),
            _ => PropertyValue::String(value.to_string()),
        }
    }

    /// 隣接リスト形式のJSONからグラフをインポート
    pub fn import_adjacency<R: Read>(graph: &mut Graph, reader: R) -> Result<ImportStats> {
        let adj_graph: AdjacencyGraph = serde_json::from_reader(reader)?;

        let mut id_map: HashMap<String, u64> = HashMap::new();

        // 最初にすべてのノードを作成
        for adj_node in &adj_graph.nodes {
            let labels: Vec<String> = if adj_node.label.is_empty() {
                vec![]
            } else {
                adj_node
                    .label
                    .split(':')
                    .map(|s| s.to_string())
                    .collect()
            };
            let node_id = graph.create_node_with_labels(labels);
            id_map.insert(adj_node.id.clone(), node_id);

            if let Some(node) = graph.get_node_mut(node_id) {
                for (key, value) in &adj_node.properties {
                    node.set_property(key.clone(), Self::json_to_property(value));
                }
            }
        }

        // エッジを作成
        let mut edges_imported = 0;
        for adj_node in &adj_graph.nodes {
            let from_id = *id_map.get(&adj_node.id).unwrap();

            for adj_edge in &adj_node.edges {
                let to_id = id_map.get(&adj_edge.to).copied().ok_or_else(|| {
                    IoError::InvalidFormat(format!("Unknown node ID: {}", adj_edge.to))
                })?;

                let edge_id = graph.create_edge(from_id, to_id, &adj_edge.edge_type)?;

                if let Some(edge) = graph.get_edge_mut(edge_id) {
                    for (key, value) in &adj_edge.properties {
                        edge.set_property(key.clone(), Self::json_to_property(value));
                    }
                }

                edges_imported += 1;
            }
        }

        Ok(ImportStats {
            nodes_imported: adj_graph.nodes.len(),
            edges_imported,
        })
    }
}

/// インポート統計
#[derive(Debug, Clone)]
pub struct ImportStats {
    pub nodes_imported: usize,
    pub edges_imported: usize,
}

/// JSONエクスポーター
pub struct JsonExporter;

impl JsonExporter {
    /// グラフをJSONにエクスポート
    pub fn export<W: Write>(graph: &Graph, writer: W) -> Result<()> {
        let json_graph = Self::to_json_graph(graph);
        serde_json::to_writer_pretty(writer, &json_graph)?;
        Ok(())
    }

    /// グラフをJSON文字列にエクスポート
    pub fn export_to_string(graph: &Graph) -> Result<String> {
        let json_graph = Self::to_json_graph(graph);
        Ok(serde_json::to_string_pretty(&json_graph)?)
    }

    fn to_json_graph(graph: &Graph) -> JsonGraph {
        let nodes: Vec<JsonNode> = graph
            .nodes()
            .map(|node| JsonNode {
                id: node.id.to_string(),
                // Join labels as colon-separated for serialization
                label: node.labels.join(":"),
                properties: node
                    .properties
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::property_to_json(v)))
                    .collect(),
            })
            .collect();

        let edges: Vec<JsonEdge> = graph
            .edges()
            .map(|edge| JsonEdge {
                from: edge.from.to_string(),
                to: edge.to.to_string(),
                edge_type: edge.label.clone(),
                properties: edge
                    .properties
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::property_to_json(v)))
                    .collect(),
            })
            .collect();

        JsonGraph { nodes, edges }
    }

    fn property_to_json(value: &PropertyValue) -> serde_json::Value {
        match value {
            PropertyValue::Null => serde_json::Value::Null,
            PropertyValue::Bool(b) => serde_json::Value::Bool(*b),
            PropertyValue::Int(n) => serde_json::Value::Number((*n).into()),
            PropertyValue::Float(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        }
    }

    /// グラフを隣接リスト形式のJSONにエクスポート
    pub fn export_adjacency<W: Write>(graph: &Graph, writer: W) -> Result<()> {
        let adj_graph = Self::to_adjacency_graph(graph);
        serde_json::to_writer_pretty(writer, &adj_graph)?;
        Ok(())
    }

    /// グラフを隣接リスト形式のJSON文字列にエクスポート
    pub fn export_adjacency_to_string(graph: &Graph) -> Result<String> {
        let adj_graph = Self::to_adjacency_graph(graph);
        Ok(serde_json::to_string_pretty(&adj_graph)?)
    }

    fn to_adjacency_graph(graph: &Graph) -> AdjacencyGraph {
        let nodes: Vec<AdjacencyNode> = graph
            .nodes()
            .map(|node| {
                // このノードから出ているエッジを収集
                let edges: Vec<AdjacencyEdge> = graph
                    .edges()
                    .filter(|edge| edge.from == node.id)
                    .map(|edge| AdjacencyEdge {
                        to: edge.to.to_string(),
                        edge_type: edge.label.clone(),
                        properties: edge
                            .properties
                            .iter()
                            .map(|(k, v)| (k.clone(), Self::property_to_json(v)))
                            .collect(),
                    })
                    .collect();

                AdjacencyNode {
                    id: node.id.to_string(),
                    label: node.labels.join(":"),
                    properties: node
                        .properties
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::property_to_json(v)))
                        .collect(),
                    edges,
                }
            })
            .collect();

        AdjacencyGraph { nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_json() {
        let json_data = r#"{
            "nodes": [
                {"id": "1", "label": "Person", "properties": {"name": "Alice", "age": 30}},
                {"id": "2", "label": "Person", "properties": {"name": "Bob", "age": 25}}
            ],
            "edges": [
                {"from": "1", "to": "2", "type": "KNOWS", "properties": {"since": 2020}}
            ]
        }"#;

        let mut graph = Graph::new();
        let stats = JsonImporter::import(&mut graph, json_data.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(stats.edges_imported, 1);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_export_json() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(a) {
            node.set_property("name", "Alice");
        }
        if let Some(node) = graph.get_node_mut(b) {
            node.set_property("name", "Bob");
        }

        graph.create_edge(a, b, "KNOWS").unwrap();

        let json_str = JsonExporter::export_to_string(&graph).unwrap();

        assert!(json_str.contains("\"nodes\""));
        assert!(json_str.contains("\"edges\""));
        assert!(json_str.contains("Person"));
        assert!(json_str.contains("KNOWS"));
    }

    #[test]
    fn test_roundtrip() {
        // 元のグラフを作成
        let mut graph1 = Graph::new();
        let a = graph1.create_node("Person");
        let b = graph1.create_node("Company");

        if let Some(node) = graph1.get_node_mut(a) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }
        if let Some(node) = graph1.get_node_mut(b) {
            node.set_property("name", "TechCorp");
        }

        graph1.create_edge(a, b, "WORKS_AT").unwrap();

        // エクスポート
        let mut json_output = Vec::new();
        JsonExporter::export(&graph1, &mut json_output).unwrap();

        // インポート
        let mut graph2 = Graph::new();
        JsonImporter::import(&mut graph2, json_output.as_slice()).unwrap();

        assert_eq!(graph2.node_count(), 2);
        assert_eq!(graph2.edge_count(), 1);
    }

    #[test]
    fn test_empty_properties() {
        let json_data = r#"{
            "nodes": [
                {"id": "1", "label": "Node"}
            ],
            "edges": []
        }"#;

        let mut graph = Graph::new();
        let stats = JsonImporter::import(&mut graph, json_data.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 1);
        assert_eq!(graph.node_count(), 1);
    }

    // ========== 隣接リスト形式のテスト ==========

    #[test]
    fn test_import_adjacency_json() {
        let json_data = r#"{
            "nodes": [
                {
                    "id": "1",
                    "label": "Person",
                    "properties": {"name": "Alice", "age": 30},
                    "edges": [
                        {"to": "2", "type": "KNOWS", "properties": {"since": 2020}}
                    ]
                },
                {
                    "id": "2",
                    "label": "Person",
                    "properties": {"name": "Bob", "age": 25},
                    "edges": []
                }
            ]
        }"#;

        let mut graph = Graph::new();
        let stats = JsonImporter::import_adjacency(&mut graph, json_data.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(stats.edges_imported, 1);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_export_adjacency_json() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(a) {
            node.set_property("name", "Alice");
        }
        if let Some(node) = graph.get_node_mut(b) {
            node.set_property("name", "Bob");
        }

        graph.create_edge(a, b, "KNOWS").unwrap();

        let json_str = JsonExporter::export_adjacency_to_string(&graph).unwrap();

        assert!(json_str.contains("\"nodes\""));
        assert!(json_str.contains("\"edges\""));
        assert!(json_str.contains("Person"));
        assert!(json_str.contains("KNOWS"));
        // 隣接リスト形式では各ノードにedgesフィールドがある
        assert!(json_str.contains("\"to\""));
    }

    #[test]
    fn test_adjacency_roundtrip() {
        // 元のグラフを作成
        let mut graph1 = Graph::new();
        let a = graph1.create_node("Person");
        let b = graph1.create_node("Company");
        let c = graph1.create_node("Person");

        if let Some(node) = graph1.get_node_mut(a) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }
        if let Some(node) = graph1.get_node_mut(b) {
            node.set_property("name", "TechCorp");
        }
        if let Some(node) = graph1.get_node_mut(c) {
            node.set_property("name", "Bob");
        }

        graph1.create_edge(a, b, "WORKS_AT").unwrap();
        graph1.create_edge(a, c, "KNOWS").unwrap();

        // エクスポート
        let mut json_output = Vec::new();
        JsonExporter::export_adjacency(&graph1, &mut json_output).unwrap();

        // インポート
        let mut graph2 = Graph::new();
        JsonImporter::import_adjacency(&mut graph2, json_output.as_slice()).unwrap();

        assert_eq!(graph2.node_count(), 3);
        assert_eq!(graph2.edge_count(), 2);
    }

    #[test]
    fn test_adjacency_empty_edges() {
        let json_data = r#"{
            "nodes": [
                {"id": "1", "label": "Node"},
                {"id": "2", "label": "Node"}
            ]
        }"#;

        let mut graph = Graph::new();
        let stats = JsonImporter::import_adjacency(&mut graph, json_data.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(stats.edges_imported, 0);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
    }
}
