use std::fmt::Write;

use maharit_core::{Graph, PropertyValue};

/// DOTスタイル設定
#[derive(Debug, Clone)]
pub struct DotStyle {
    /// グラフの方向（true: 上から下、false: 左から右）
    pub top_to_bottom: bool,
    /// ノードの形状
    pub node_shape: String,
    /// ノードの色
    pub node_color: String,
    /// エッジの色
    pub edge_color: String,
    /// プロパティを表示するか
    pub show_properties: bool,
    /// 表示するプロパティの最大数
    pub max_properties: usize,
}

impl Default for DotStyle {
    fn default() -> Self {
        Self {
            top_to_bottom: true,
            node_shape: "ellipse".to_string(),
            node_color: "lightblue".to_string(),
            edge_color: "gray".to_string(),
            show_properties: true,
            max_properties: 3,
        }
    }
}

/// DOT形式エクスポーター
pub struct DotExporter;

impl DotExporter {
    /// グラフをDOT形式に変換
    pub fn export(graph: &Graph) -> String {
        Self::export_with_style(graph, &DotStyle::default())
    }

    /// スタイル指定付きでDOT形式に変換
    pub fn export_with_style(graph: &Graph, style: &DotStyle) -> String {
        let mut dot = String::new();

        // ヘッダー
        writeln!(dot, "digraph G {{").unwrap();

        // グラフ属性
        let rankdir = if style.top_to_bottom { "TB" } else { "LR" };
        writeln!(dot, "    rankdir={};", rankdir).unwrap();
        writeln!(
            dot,
            "    node [shape={}, style=filled, fillcolor={}];",
            style.node_shape, style.node_color
        )
        .unwrap();
        writeln!(dot, "    edge [color={}];", style.edge_color).unwrap();
        writeln!(dot).unwrap();

        // ノード
        writeln!(dot, "    // Nodes").unwrap();
        for node in graph.nodes() {
            let label = Self::make_node_label(node, style);
            writeln!(
                dot,
                "    n{} [label=\"{}\"];",
                node.id,
                Self::escape(&label)
            )
            .unwrap();
        }
        writeln!(dot).unwrap();

        // エッジ
        writeln!(dot, "    // Edges").unwrap();
        for edge in graph.edges() {
            let label = if edge.label.is_empty() {
                String::new()
            } else {
                format!(" [label=\"{}\"]", Self::escape(&edge.label))
            };
            writeln!(dot, "    n{} -> n{}{};", edge.from, edge.to, label).unwrap();
        }

        writeln!(dot, "}}").unwrap();

        dot
    }

    fn make_node_label(node: &maharit_core::Node, style: &DotStyle) -> String {
        let mut label = if node.label.is_empty() {
            format!("({})", node.id)
        } else {
            format!("{}:{}", node.id, node.label)
        };

        if style.show_properties && !node.properties.is_empty() {
            label.push_str("\\n");
            let props: Vec<String> = node
                .properties
                .iter()
                .take(style.max_properties)
                .map(|(k, v)| format!("{}: {}", k, Self::format_value(v)))
                .collect();
            label.push_str(&props.join("\\n"));

            if node.properties.len() > style.max_properties {
                label.push_str(&format!(
                    "\\n... +{} more",
                    node.properties.len() - style.max_properties
                ));
            }
        }

        label
    }

    fn format_value(v: &PropertyValue) -> String {
        match v {
            PropertyValue::Null => "null".to_string(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Int(n) => n.to_string(),
            PropertyValue::Float(n) => format!("{:.2}", n),
            PropertyValue::String(s) => {
                if s.len() > 20 {
                    format!("{}...", &s[..17])
                } else {
                    s.clone()
                }
            }
        }
    }

    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_simple() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");
        graph.create_edge(a, b, "KNOWS").unwrap();

        let dot = DotExporter::export(&graph);

        assert!(dot.contains("digraph G {"));
        assert!(dot.contains("n0"));
        assert!(dot.contains("n1"));
        assert!(dot.contains("n0 -> n1"));
        assert!(dot.contains("KNOWS"));
    }

    #[test]
    fn test_export_with_properties() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }

        let dot = DotExporter::export(&graph);

        assert!(dot.contains("Person"));
        // プロパティが含まれている
        assert!(dot.contains("name") || dot.contains("age"));
    }

    #[test]
    fn test_export_without_properties() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("name", "Alice");
        }

        let style = DotStyle {
            show_properties: false,
            ..Default::default()
        };
        let dot = DotExporter::export_with_style(&graph, &style);

        // プロパティが含まれていない
        assert!(!dot.contains("Alice"));
    }

    #[test]
    fn test_escape_special_chars() {
        let mut graph = Graph::new();
        let id = graph.create_node("Test");
        if let Some(node) = graph.get_node_mut(id) {
            node.set_property("desc", "Line1\nLine2");
        }

        let dot = DotExporter::export(&graph);

        // 改行がエスケープされている
        assert!(dot.contains("\\n"));
    }
}
