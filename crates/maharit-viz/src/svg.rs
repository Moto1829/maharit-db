//! SVG export with force-directed layout
//!
//! Implements the Fruchterman-Reingold force-directed algorithm for positioning
//! graph nodes, then renders the result as an SVG document.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use maharit_core::Graph;

/// A computed (x, y) position for a single node.
#[derive(Debug, Clone)]
pub struct NodePosition {
    pub node_id: u64,
    pub x: f64,
    pub y: f64,
}

/// Parameters for the Fruchterman-Reingold force-directed layout algorithm.
#[derive(Debug, Clone)]
pub struct ForceDirectedLayout {
    /// Number of simulation steps (higher = more stable layout).
    pub iterations: usize,
    /// Coulomb-like repulsion constant between all node pairs.
    pub repulsion: f64,
    /// Hooke-like attraction constant along edges.
    pub attraction: f64,
    /// Velocity damping factor applied each iteration (0 < damping < 1).
    pub damping: f64,
    /// Width of the output canvas in pixels.
    pub canvas_width: f64,
    /// Height of the output canvas in pixels.
    pub canvas_height: f64,
}

impl Default for ForceDirectedLayout {
    fn default() -> Self {
        Self {
            iterations: 100,
            repulsion: 500.0,
            attraction: 0.1,
            damping: 0.85,
            canvas_width: 800.0,
            canvas_height: 600.0,
        }
    }
}

impl ForceDirectedLayout {
    /// Compute node positions using the Fruchterman-Reingold algorithm.
    ///
    /// Returns a `Vec<NodePosition>` with one entry per node. When the graph
    /// is empty the vector is empty. Single-node graphs return the canvas
    /// centre.
    pub fn compute(&self, graph: &Graph) -> Vec<NodePosition> {
        let nodes: Vec<u64> = graph.nodes().map(|n| n.id).collect();
        let n = nodes.len();
        if n == 0 {
            return Vec::new();
        }

        // Build a fast index: node_id -> index into positions array.
        let index: HashMap<u64, usize> = nodes.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // --- Initial positions on a circle ---
        let cx = self.canvas_width / 2.0;
        let cy = self.canvas_height / 2.0;
        let radius = (self.canvas_width.min(self.canvas_height) / 2.0) * 0.8;

        let mut pos_x: Vec<f64> = (0..n)
            .map(|i| {
                let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                cx + radius * angle.cos()
            })
            .collect();
        let mut pos_y: Vec<f64> = (0..n)
            .map(|i| {
                let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                cy + radius * angle.sin()
            })
            .collect();

        // For a single node just place it at the centre.
        if n == 1 {
            pos_x[0] = cx;
            pos_y[0] = cy;
            return vec![NodePosition {
                node_id: nodes[0],
                x: cx,
                y: cy,
            }];
        }

        // Velocity buffers.
        let mut vel_x = vec![0.0f64; n];
        let mut vel_y = vec![0.0f64; n];

        // Padding so nodes do not touch the canvas border.
        let padding = 20.0;

        for _ in 0..self.iterations {
            // Accumulate forces for this step.
            let mut fx = vec![0.0f64; n];
            let mut fy = vec![0.0f64; n];

            // Repulsion: all pairs (O(n²)).
            for i in 0..n {
                for j in (i + 1)..n {
                    let dx = pos_x[i] - pos_x[j];
                    let dy = pos_y[i] - pos_y[j];
                    let dist_sq = (dx * dx + dy * dy).max(1.0); // avoid div-by-zero
                    let dist = dist_sq.sqrt();
                    let force = self.repulsion / dist_sq;
                    let ux = dx / dist;
                    let uy = dy / dist;
                    fx[i] += force * ux;
                    fy[i] += force * uy;
                    fx[j] -= force * ux;
                    fy[j] -= force * uy;
                }
            }

            // Attraction: along edges.
            for edge in graph.edges() {
                let Some(&i) = index.get(&edge.from) else {
                    continue;
                };
                let Some(&j) = index.get(&edge.to) else {
                    continue;
                };
                let dx = pos_x[j] - pos_x[i];
                let dy = pos_y[j] - pos_y[i];
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let force = self.attraction * dist;
                let ux = dx / dist;
                let uy = dy / dist;
                fx[i] += force * ux;
                fy[i] += force * uy;
                fx[j] -= force * ux;
                fy[j] -= force * uy;
            }

            // Update velocities with damping, then positions.
            for i in 0..n {
                vel_x[i] = (vel_x[i] + fx[i]) * self.damping;
                vel_y[i] = (vel_y[i] + fy[i]) * self.damping;
                pos_x[i] += vel_x[i];
                pos_y[i] += vel_y[i];

                // Clamp to canvas.
                pos_x[i] = pos_x[i].clamp(padding, self.canvas_width - padding);
                pos_y[i] = pos_y[i].clamp(padding, self.canvas_height - padding);
            }
        }

        nodes
            .iter()
            .enumerate()
            .map(|(i, &id)| NodePosition {
                node_id: id,
                x: pos_x[i],
                y: pos_y[i],
            })
            .collect()
    }
}

/// SVG exporter that renders a graph using a force-directed layout.
pub struct SvgExporter {
    /// Layout algorithm configuration.
    pub layout: ForceDirectedLayout,
    /// Radius of node circles in pixels.
    pub node_radius: f64,
    /// Fill colour for node circles (CSS colour string).
    pub node_color: String,
    /// Stroke colour for edges (CSS colour string).
    pub edge_color: String,
    /// Font size for node labels in pixels.
    pub font_size: f64,
}

impl Default for SvgExporter {
    fn default() -> Self {
        Self {
            layout: ForceDirectedLayout::default(),
            node_radius: 20.0,
            node_color: "#4a90e2".to_string(),
            edge_color: "#999".to_string(),
            font_size: 12.0,
        }
    }
}

impl SvgExporter {
    /// Render the graph as an SVG string.
    ///
    /// The returned string is a complete, self-contained SVG document.
    pub fn export(&self, graph: &Graph) -> String {
        let positions = self.layout.compute(graph);

        // Build a lookup: node_id -> (x, y).
        let pos_map: HashMap<u64, (f64, f64)> = positions
            .iter()
            .map(|p| (p.node_id, (p.x, p.y)))
            .collect();

        let w = self.layout.canvas_width as u64;
        let h = self.layout.canvas_height as u64;

        let mut svg = String::new();

        writeln!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}">"#
        )
        .unwrap();

        // --- Edges (drawn first so they appear beneath nodes) ---
        for edge in graph.edges() {
            let Some(&(x1, y1)) = pos_map.get(&edge.from) else {
                continue;
            };
            let Some(&(x2, y2)) = pos_map.get(&edge.to) else {
                continue;
            };
            writeln!(
                svg,
                r#"  <line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{color}" stroke-width="2"/>"#,
                color = self.edge_color,
            )
            .unwrap();

            // Optional edge label at the midpoint.
            if !edge.label.is_empty() {
                let mx = (x1 + x2) / 2.0;
                let my = (y1 + y2) / 2.0;
                let escaped = escape_xml(&edge.label);
                let edge_fs = self.font_size - 2.0;
                writeln!(
                    svg,
                    "  <text x=\"{mx:.1}\" y=\"{my:.1}\" text-anchor=\"middle\" font-size=\"{edge_fs}\" fill=\"#666\">{escaped}</text>",
                )
                .unwrap();
            }
        }

        // --- Nodes ---
        for node in graph.nodes() {
            let Some(&(cx, cy)) = pos_map.get(&node.id) else {
                continue;
            };
            writeln!(
                svg,
                r#"  <circle cx="{cx:.1}" cy="{cy:.1}" r="{r}" fill="{color}"/>"#,
                r = self.node_radius,
                color = self.node_color,
            )
            .unwrap();

            // Label: "label:id" or just the id when no label is set.
            let raw_label = if node.labels.is_empty() {
                node.id.to_string()
            } else {
                format!(
                    "{}:{}",
                    node.labels.join(":"),
                    node.id
                )
            };
            let escaped = escape_xml(&raw_label);
            writeln!(
                svg,
                r#"  <text x="{cx:.1}" y="{ty:.1}" text-anchor="middle" font-size="{fs}">{escaped}</text>"#,
                ty = cy + self.font_size / 3.0,
                fs = self.font_size,
            )
            .unwrap();
        }

        writeln!(svg, "</svg>").unwrap();
        svg
    }

    /// Write the SVG to a file at `path`.
    pub fn export_to_file(&self, graph: &Graph, path: &str) -> std::io::Result<()> {
        let svg = self.export(graph);
        std::fs::write(path, svg)
    }
}

/// Escape characters that have special meaning in XML/SVG text content.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_empty_graph_produces_svg_tags() {
        let graph = Graph::new();
        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        // Must at least open and close the <svg> element.
        assert!(svg.contains("<svg"), "missing opening <svg> tag");
        assert!(svg.contains("</svg>"), "missing closing </svg> tag");
        // No circles for an empty graph.
        assert!(!svg.contains("<circle"), "empty graph should have no circles");
    }

    #[test]
    fn test_export_single_node_contains_circle() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        assert!(svg.contains("<circle"), "single node should produce a <circle>");
    }

    #[test]
    fn test_export_nodes_contain_circle_tags() {
        let mut graph = Graph::new();
        graph.create_node("Person");
        graph.create_node("Movie");
        graph.create_node("City");
        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        // Count circle elements.
        let circle_count = svg.matches("<circle").count();
        assert_eq!(circle_count, 3, "expected one <circle> per node");
    }

    #[test]
    fn test_export_edge_contains_line_tag() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        graph.create_edge(a, b, "KNOWS").unwrap();

        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        assert!(svg.contains("<line"), "edge should produce a <line> element");
    }

    #[test]
    fn test_export_no_edges_no_line_tag() {
        let mut graph = Graph::new();
        graph.create_node("Isolated");

        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        assert!(!svg.contains("<line"), "no edges means no <line> tags");
    }

    #[test]
    fn test_layout_positions_within_canvas() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        graph.create_edge(a, b, "e").unwrap();
        graph.create_edge(b, c, "e").unwrap();

        let layout = ForceDirectedLayout::default();
        let positions = layout.compute(&graph);

        assert_eq!(positions.len(), 3);
        for p in &positions {
            assert!(
                p.x >= 0.0 && p.x <= layout.canvas_width,
                "x={} out of canvas width {}",
                p.x,
                layout.canvas_width
            );
            assert!(
                p.y >= 0.0 && p.y <= layout.canvas_height,
                "y={} out of canvas height {}",
                p.y,
                layout.canvas_height
            );
        }
    }

    #[test]
    fn test_layout_empty_graph_returns_empty_vec() {
        let graph = Graph::new();
        let layout = ForceDirectedLayout::default();
        let positions = layout.compute(&graph);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_export_to_file_creates_file() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        graph.create_edge(a, b, "REL").unwrap();

        let exporter = SvgExporter::default();
        let path = "/tmp/maharit_test_export.svg";
        exporter.export_to_file(&graph, path).expect("export_to_file failed");

        let contents = std::fs::read_to_string(path).expect("file not found after export");
        assert!(contents.contains("<svg"), "exported file lacks <svg> tag");
        assert!(contents.contains("<circle"), "exported file lacks <circle>");
        assert!(contents.contains("<line"), "exported file lacks <line>");

        // Cleanup.
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_svg_contains_canvas_dimensions() {
        let graph = Graph::new();
        let exporter = SvgExporter {
            layout: ForceDirectedLayout {
                canvas_width: 1024.0,
                canvas_height: 768.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let svg = exporter.export(&graph);
        assert!(svg.contains("width=\"1024\""));
        assert!(svg.contains("height=\"768\""));
    }

    #[test]
    fn test_node_label_appears_in_svg() {
        let mut graph = Graph::new();
        graph.create_node("Person");

        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        assert!(svg.contains("Person"), "node label should appear in SVG text");
    }

    #[test]
    fn test_xml_special_chars_are_escaped() {
        let mut graph = Graph::new();
        graph.create_node("<Script>&");

        let exporter = SvgExporter::default();
        let svg = exporter.export(&graph);

        // Raw < and & must not appear inside element content.
        // We allow the SVG tag attributes themselves; check only text content.
        assert!(svg.contains("&lt;"), "< should be escaped to &lt;");
        assert!(svg.contains("&amp;"), "& should be escaped to &amp;");
    }
}
