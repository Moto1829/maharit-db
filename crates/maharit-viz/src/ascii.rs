use std::collections::{HashSet, VecDeque};
use std::fmt::Write;

use maharit_core::{Graph, NodeId};

/// ASCII形式でグラフを表示するレンダラー
pub struct AsciiRenderer;

impl AsciiRenderer {
    /// グラフ全体をASCIIで表示
    pub fn render(graph: &Graph) -> String {
        let mut output = String::new();

        if graph.node_count() == 0 {
            return "(empty graph)".to_string();
        }

        writeln!(
            output,
            "Graph: {} nodes, {} edges",
            graph.node_count(),
            graph.edge_count()
        )
        .unwrap();
        writeln!(output).unwrap();

        // 各ノードとその接続を表示
        for node in graph.nodes() {
            // ノード自体
            write!(output, "({})", node.id).unwrap();
            if !node.labels.is_empty() {
                write!(output, "{}", node.labels.iter().map(|l| format!(":{}", l)).collect::<String>()).unwrap();
            }
            writeln!(output).unwrap();

            // 出るエッジ
            let outgoing: Vec<_> = graph.get_outgoing_edges(node.id).collect();
            for (i, edge) in outgoing.iter().enumerate() {
                let is_last = i == outgoing.len() - 1;
                let prefix = if is_last { "└" } else { "├" };
                writeln!(output, " {}──[:{}]──> ({})", prefix, edge.label, edge.to).unwrap();
            }

            if !outgoing.is_empty() {
                writeln!(output).unwrap();
            }
        }

        output
    }

    /// 指定したノードからツリー状に表示
    pub fn render_tree(graph: &Graph, root: NodeId, max_depth: usize) -> String {
        let mut output = String::new();
        let mut visited = HashSet::new();

        Self::render_node_tree(
            graph,
            root,
            0,
            max_depth,
            "",
            true,
            &mut visited,
            &mut output,
        );

        output
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node_tree(
        graph: &Graph,
        node_id: NodeId,
        depth: usize,
        max_depth: usize,
        prefix: &str,
        is_last: bool,
        visited: &mut HashSet<NodeId>,
        output: &mut String,
    ) {
        if depth > max_depth {
            return;
        }

        let Some(node) = graph.get_node(node_id) else {
            return;
        };

        // 現在のノードを表示
        let connector = if depth == 0 {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        write!(output, "{}{}", prefix, connector).unwrap();
        write!(output, "({})", node.id).unwrap();
        if !node.labels.is_empty() {
            write!(output, "{}", node.labels.iter().map(|l| format!(":{}", l)).collect::<String>()).unwrap();
        }

        // 循環検出
        if visited.contains(&node_id) {
            writeln!(output, " [circular]").unwrap();
            return;
        }
        writeln!(output).unwrap();

        visited.insert(node_id);

        // 子ノードを表示
        let outgoing: Vec<_> = graph.get_outgoing_edges(node_id).collect();
        let child_prefix = if depth == 0 {
            String::new()
        } else {
            format!("{}{}", prefix, if is_last { "    " } else { "│   " })
        };

        for (i, edge) in outgoing.iter().enumerate() {
            let is_last_child = i == outgoing.len() - 1;

            // エッジラベルを表示
            let edge_connector = if is_last_child { "└" } else { "├" };
            writeln!(
                output,
                "{}{}─[:{}]─>",
                child_prefix, edge_connector, edge.label
            )
            .unwrap();

            // 子ノードの新しいprefix
            let next_prefix = format!(
                "{}{}",
                child_prefix,
                if is_last_child { "    " } else { "│   " }
            );

            Self::render_node_tree(
                graph,
                edge.to,
                depth + 1,
                max_depth,
                &next_prefix,
                true,
                visited,
                output,
            );
        }
    }

    /// BFSで到達可能なノードをレイヤー表示
    pub fn render_layers(graph: &Graph, root: NodeId, max_depth: usize) -> String {
        let mut output = String::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back((root, 0));
        visited.insert(root);

        let mut current_depth = 0;

        while let Some((node_id, depth)) = queue.pop_front() {
            if depth > max_depth {
                continue;
            }

            // 新しいレイヤーの開始
            if depth > current_depth {
                writeln!(output).unwrap();
                current_depth = depth;
            }

            if depth == 0 {
                writeln!(output, "Layer 0 (root):").unwrap();
            } else if depth > current_depth - 1
                || (depth == current_depth && current_depth != depth)
            {
                writeln!(output, "Layer {}:", depth).unwrap();
            }

            // ノードを表示
            if let Some(node) = graph.get_node(node_id) {
                write!(output, "  ({})", node.id).unwrap();
                if !node.labels.is_empty() {
                    write!(output, "{}", node.labels.iter().map(|l| format!(":{}", l)).collect::<String>()).unwrap();
                }
                writeln!(output).unwrap();
            }

            // 隣接ノードをキューに追加
            for edge in graph.get_outgoing_edges(node_id) {
                if !visited.contains(&edge.to) {
                    visited.insert(edge.to);
                    queue.push_back((edge.to, depth + 1));
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> Graph {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");

        graph.create_edge(a, b, "to").unwrap();
        graph.create_edge(a, c, "to").unwrap();
        graph.create_edge(b, d, "to").unwrap();

        graph
    }

    #[test]
    fn test_render() {
        let graph = create_test_graph();
        let output = AsciiRenderer::render(&graph);

        assert!(output.contains("4 nodes"));
        assert!(output.contains("3 edges"));
    }

    #[test]
    fn test_render_tree() {
        let graph = create_test_graph();
        let output = AsciiRenderer::render_tree(&graph, 0, 3);

        assert!(output.contains("(0):A"));
        assert!(output.contains("[:to]"));
    }

    #[test]
    fn test_render_empty_graph() {
        let graph = Graph::new();
        let output = AsciiRenderer::render(&graph);

        assert_eq!(output, "(empty graph)");
    }

    #[test]
    fn test_circular_detection() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");

        graph.create_edge(a, b, "to").unwrap();
        graph.create_edge(b, a, "back").unwrap();

        let output = AsciiRenderer::render_tree(&graph, a, 5);

        assert!(output.contains("[circular]"));
    }

    #[test]
    fn test_render_layers() {
        let graph = create_test_graph();
        let output = AsciiRenderer::render_layers(&graph, 0, 3);

        assert!(output.contains("Layer 0"));
        assert!(output.contains("(0):A"));
    }
}
