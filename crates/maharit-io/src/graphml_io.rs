//! GraphML format import/export
//!
//! GraphML is an XML-based file format for graphs.
//! See: http://graphml.graphdrawing.org/

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};

use maharit_core::{Graph, PropertyValue};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer, XmlVersion};

use crate::{IoError, Result};

/// Type alias for a list of (property-name, type-string) pairs used in GraphML key collection.
type KeyList = Vec<(String, &'static str)>;

/// GraphML key definition
#[derive(Debug, Clone)]
struct KeyDef {
    id: String,
    name: String,
    #[allow(dead_code)]
    for_type: KeyFor,
    attr_type: AttrType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum KeyFor {
    Node,
    Edge,
    All,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum AttrType {
    String,
    Int,
    Long,
    Float,
    Double,
    Boolean,
}

impl AttrType {
    fn from_str(s: &str) -> Self {
        match s {
            "int" => AttrType::Int,
            "long" => AttrType::Long,
            "float" => AttrType::Float,
            "double" => AttrType::Double,
            "boolean" => AttrType::Boolean,
            _ => AttrType::String,
        }
    }

    #[allow(dead_code)]
    fn to_str(self) -> &'static str {
        match self {
            AttrType::String => "string",
            AttrType::Int => "int",
            AttrType::Long => "long",
            AttrType::Float => "float",
            AttrType::Double => "double",
            AttrType::Boolean => "boolean",
        }
    }
}

/// Import statistics
#[derive(Debug, Clone)]
pub struct ImportStats {
    pub nodes_imported: usize,
    pub edges_imported: usize,
}

/// Element state while streaming a GraphML document.
#[derive(Default)]
struct ImportState {
    keys: HashMap<String, KeyDef>,
    id_map: HashMap<String, u64>,
    nodes_imported: usize,
    edges_imported: usize,
    /// `id` of the `<node>` being read.
    node_id: Option<String>,
    /// `(source, target)` of the `<edge>` being read.
    edge: Option<(String, String)>,
    /// `key` of the `<data>` being read, and its text so far.
    data_key: Option<String>,
    data_text: String,
    properties: HashMap<String, PropertyValue>,
    label: Option<String>,
}

impl ImportState {
    fn start(&mut self, e: &BytesStart) {
        match e.name().as_ref() {
            "key" => {
                if let Some(key_def) = GraphMlImporter::parse_key(e) {
                    self.keys.insert(key_def.id.clone(), key_def);
                }
            }
            "node" => {
                self.node_id = GraphMlImporter::get_attr(e, "id");
                self.properties.clear();
                self.label = None;
            }
            "edge" => {
                let source = GraphMlImporter::get_attr(e, "source");
                let target = GraphMlImporter::get_attr(e, "target");
                self.edge = source.zip(target);
                self.properties.clear();
                self.label = None;
            }
            "data" => {
                self.data_key = GraphMlImporter::get_attr(e, "key");
                self.data_text.clear();
            }
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.data_key.is_some() {
            self.data_text.push_str(text);
        }
    }

    fn end(&mut self, graph: &mut Graph, name: &str) {
        match name {
            "data" => self.finish_data(),
            "node" => {
                if let Some(node_id) = self.node_id.take() {
                    let label_str = self.label.take().unwrap_or_else(|| "Node".to_string());
                    let labels: Vec<String> = label_str.split(':').map(str::to_string).collect();
                    let internal_id = graph.create_node_with_labels(labels);
                    self.id_map.insert(node_id, internal_id);
                    if let Some(node) = graph.get_node_mut(internal_id) {
                        for (k, v) in self.properties.drain() {
                            node.set_property(k, v);
                        }
                    }
                    self.nodes_imported += 1;
                }
            }
            "edge" => {
                if let Some((source, target)) = self.edge.take() {
                    let from = self.id_map.get(&source).copied();
                    let to = self.id_map.get(&target).copied();
                    if let (Some(from), Some(to)) = (from, to) {
                        let label = self.label.take().unwrap_or_else(|| "EDGE".to_string());
                        if let Ok(edge_id) = graph.create_edge(from, to, &label) {
                            if let Some(edge) = graph.get_edge_mut(edge_id) {
                                for (k, v) in self.properties.drain() {
                                    edge.set_property(k, v);
                                }
                            }
                            self.edges_imported += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `</data>`: store the collected text as a label or a typed property.
    fn finish_data(&mut self) {
        let Some(key_id) = self.data_key.take() else {
            return;
        };
        if self.node_id.is_none() && self.edge.is_none() {
            return;
        }
        let text = self.data_text.trim();
        let key_def = self.keys.get(&key_id);
        let name = key_def.map_or(key_id.as_str(), |k| k.name.as_str());
        if matches!(name, "label" | "labelV" | "labelE") {
            self.label = Some(text.to_string());
        } else {
            let value = GraphMlImporter::parse_value(text, key_def);
            self.properties.insert(name.to_string(), value);
        }
    }
}

/// GraphML importer
pub struct GraphMlImporter;

impl GraphMlImporter {
    /// Import a graph from GraphML format
    pub fn import<R: Read>(graph: &mut Graph, reader: R) -> Result<ImportStats> {
        let mut xml_reader = Reader::from_reader(BufReader::new(reader));
        // Text is not trimmed per event: entity references arrive as separate
        // events, so `a &amp; b` is Text("a "), GeneralRef("amp"), Text(" b").
        // The accumulated <data> value is trimmed once at </data> instead.
        let mut state = ImportState::default();
        let mut buf = Vec::new();

        loop {
            let event = xml_reader
                .read_event_into(&mut buf)
                .map_err(|e| IoError::InvalidFormat(format!("XML parse error: {}", e)))?;
            match event {
                Event::Start(ref e) => state.start(e),
                Event::Empty(ref e) => {
                    // Self-closing tag: a start immediately followed by its end.
                    state.start(e);
                    state.end(graph, e.name().as_ref());
                }
                Event::End(ref e) => state.end(graph, e.name().as_ref()),
                Event::Text(ref e) => state.push_text(&e.xml10_content()),
                Event::CData(ref e) => state.push_text(&e.xml10_content()),
                Event::GeneralRef(ref r) => {
                    let resolved = match r.resolve_char_ref() {
                        Ok(Some(ch)) => ch.to_string(),
                        Ok(None) => resolve_predefined_entity(r)
                            .ok_or_else(|| {
                                IoError::InvalidFormat(format!("unknown XML entity: &{};", &**r))
                            })?
                            .to_string(),
                        Err(e) => {
                            return Err(IoError::InvalidFormat(format!(
                                "invalid XML character reference: {}",
                                e
                            )));
                        }
                    };
                    state.push_text(&resolved);
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }

        Ok(ImportStats {
            nodes_imported: state.nodes_imported,
            edges_imported: state.edges_imported,
        })
    }

    fn parse_key(e: &BytesStart) -> Option<KeyDef> {
        let id = Self::get_attr(e, "id")?;
        let name = Self::get_attr(e, "attr.name").unwrap_or_else(|| id.clone());
        let for_str = Self::get_attr(e, "for").unwrap_or_else(|| "all".to_string());
        let attr_type_str = Self::get_attr(e, "attr.type").unwrap_or_else(|| "string".to_string());

        let for_type = match for_str.as_str() {
            "node" => KeyFor::Node,
            "edge" => KeyFor::Edge,
            _ => KeyFor::All,
        };

        Some(KeyDef {
            id,
            name,
            for_type,
            attr_type: AttrType::from_str(&attr_type_str),
        })
    }

    fn get_attr(e: &BytesStart, name: &str) -> Option<String> {
        e.attributes()
            .filter_map(|a| a.ok())
            .find(|a| a.key.as_ref() == name)
            .and_then(|a| {
                a.normalized_value(XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
    }

    fn parse_value(text: &str, key_def: Option<&KeyDef>) -> PropertyValue {
        if let Some(key) = key_def {
            match key.attr_type {
                AttrType::Int | AttrType::Long => text
                    .parse::<i64>()
                    .map(PropertyValue::Int)
                    .unwrap_or(PropertyValue::String(text.to_string())),
                AttrType::Float | AttrType::Double => text
                    .parse::<f64>()
                    .map(PropertyValue::Float)
                    .unwrap_or(PropertyValue::String(text.to_string())),
                AttrType::Boolean => PropertyValue::Bool(text == "true" || text == "1"),
                AttrType::String => PropertyValue::String(text.to_string()),
            }
        } else {
            // Try to infer type
            if let Ok(i) = text.parse::<i64>() {
                PropertyValue::Int(i)
            } else if let Ok(f) = text.parse::<f64>() {
                PropertyValue::Float(f)
            } else if text == "true" || text == "false" {
                PropertyValue::Bool(text == "true")
            } else {
                PropertyValue::String(text.to_string())
            }
        }
    }
}

/// GraphML exporter
pub struct GraphMlExporter;

impl GraphMlExporter {
    /// Export a graph to GraphML format
    pub fn export<W: Write>(graph: &Graph, writer: W) -> Result<()> {
        let mut xml_writer = Writer::new_with_indent(writer, b' ', 2);

        // XML declaration
        xml_writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

        // GraphML root element
        let mut graphml = BytesStart::new("graphml");
        graphml.push_attribute(("xmlns", "http://graphml.graphdrawing.org/xmlns"));
        graphml.push_attribute(("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance"));
        graphml.push_attribute(("xsi:schemaLocation", "http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd"));
        xml_writer.write_event(Event::Start(graphml))?;

        // Collect all property keys
        let (node_keys, edge_keys) = Self::collect_keys(graph);

        // Write key definitions
        Self::write_key(&mut xml_writer, "label", "label", "node", "string")?;
        Self::write_key(&mut xml_writer, "labelE", "label", "edge", "string")?;

        for (name, attr_type) in &node_keys {
            Self::write_key(
                &mut xml_writer,
                &format!("n_{}", name),
                name,
                "node",
                attr_type,
            )?;
        }
        for (name, attr_type) in &edge_keys {
            Self::write_key(
                &mut xml_writer,
                &format!("e_{}", name),
                name,
                "edge",
                attr_type,
            )?;
        }

        // Graph element
        let mut graph_elem = BytesStart::new("graph");
        graph_elem.push_attribute(("id", "G"));
        graph_elem.push_attribute(("edgedefault", "directed"));
        xml_writer.write_event(Event::Start(graph_elem))?;

        // Write nodes
        for node in graph.nodes() {
            let mut node_elem = BytesStart::new("node");
            node_elem.push_attribute(("id", node.id.to_string().as_str()));
            xml_writer.write_event(Event::Start(node_elem))?;

            // Label (colon-separated for multi-label support)
            Self::write_data(&mut xml_writer, "label", &node.labels.join(":"))?;

            // Properties
            for (key, value) in node.properties.iter() {
                Self::write_data(
                    &mut xml_writer,
                    &format!("n_{}", key),
                    &Self::property_to_string(value),
                )?;
            }

            xml_writer.write_event(Event::End(BytesEnd::new("node")))?;
        }

        // Write edges
        for (idx, edge) in graph.edges().enumerate() {
            let mut edge_elem = BytesStart::new("edge");
            edge_elem.push_attribute(("id", format!("e{}", idx).as_str()));
            edge_elem.push_attribute(("source", edge.from.to_string().as_str()));
            edge_elem.push_attribute(("target", edge.to.to_string().as_str()));
            xml_writer.write_event(Event::Start(edge_elem))?;

            // Label
            Self::write_data(&mut xml_writer, "labelE", &edge.label)?;

            // Properties
            for (key, value) in edge.properties.iter() {
                Self::write_data(
                    &mut xml_writer,
                    &format!("e_{}", key),
                    &Self::property_to_string(value),
                )?;
            }

            xml_writer.write_event(Event::End(BytesEnd::new("edge")))?;
        }

        // Close graph
        xml_writer.write_event(Event::End(BytesEnd::new("graph")))?;

        // Close graphml
        xml_writer.write_event(Event::End(BytesEnd::new("graphml")))?;

        Ok(())
    }

    /// Export to string
    pub fn export_to_string(graph: &Graph) -> Result<String> {
        let mut output = Vec::new();
        Self::export(graph, &mut output)?;
        String::from_utf8(output).map_err(|e| IoError::InvalidFormat(e.to_string()))
    }

    fn collect_keys(graph: &Graph) -> (KeyList, KeyList) {
        let mut node_keys: HashMap<String, &'static str> = HashMap::new();
        let mut edge_keys: HashMap<String, &'static str> = HashMap::new();

        for node in graph.nodes() {
            for (key, value) in node.properties.iter() {
                node_keys
                    .entry(key.clone())
                    .or_insert_with(|| Self::property_type(value));
            }
        }

        for edge in graph.edges() {
            for (key, value) in edge.properties.iter() {
                edge_keys
                    .entry(key.clone())
                    .or_insert_with(|| Self::property_type(value));
            }
        }

        (
            node_keys.into_iter().collect(),
            edge_keys.into_iter().collect(),
        )
    }

    fn property_type(value: &PropertyValue) -> &'static str {
        match value {
            PropertyValue::Null => "string",
            PropertyValue::Bool(_) => "boolean",
            PropertyValue::Int(_) => "long",
            PropertyValue::Float(_) => "double",
            PropertyValue::String(_) => "string",
            PropertyValue::Date(_)
            | PropertyValue::DateTime(_)
            | PropertyValue::Duration { .. } => "string",
        }
    }

    fn write_key<W: Write>(
        writer: &mut Writer<W>,
        id: &str,
        name: &str,
        for_type: &str,
        attr_type: &str,
    ) -> Result<()> {
        let mut key = BytesStart::new("key");
        key.push_attribute(("id", id));
        key.push_attribute(("for", for_type));
        key.push_attribute(("attr.name", name));
        key.push_attribute(("attr.type", attr_type));
        writer.write_event(Event::Empty(key))?;
        Ok(())
    }

    fn write_data<W: Write>(writer: &mut Writer<W>, key: &str, value: &str) -> Result<()> {
        let mut data = BytesStart::new("data");
        data.push_attribute(("key", key));
        writer.write_event(Event::Start(data))?;
        writer.write_event(Event::Text(BytesText::new(value)))?;
        writer.write_event(Event::End(BytesEnd::new("data")))?;
        Ok(())
    }

    fn property_to_string(value: &PropertyValue) -> String {
        match value {
            PropertyValue::Null => String::new(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Int(n) => n.to_string(),
            PropertyValue::Float(n) => n.to_string(),
            PropertyValue::String(s) => s.clone(),
            PropertyValue::Date(_)
            | PropertyValue::DateTime(_)
            | PropertyValue::Duration { .. } => value.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_graphml() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");

        if let Some(node) = graph.get_node_mut(a) {
            node.set_property("name", "Alice");
            node.set_property("age", 30);
        }
        if let Some(node) = graph.get_node_mut(b) {
            node.set_property("name", "Bob");
        }

        graph.create_edge(a, b, "KNOWS").unwrap();

        let xml = GraphMlExporter::export_to_string(&graph).unwrap();

        assert!(xml.contains("graphml"));
        assert!(xml.contains("<node"));
        assert!(xml.contains("<edge"));
        assert!(xml.contains("Person"));
        assert!(xml.contains("KNOWS"));
        assert!(xml.contains("Alice"));
    }

    #[test]
    fn test_import_graphml() {
        let graphml = r#"<?xml version="1.0" encoding="UTF-8"?>
<graphml xmlns="http://graphml.graphdrawing.org/xmlns">
  <key id="label" for="node" attr.name="label" attr.type="string"/>
  <key id="labelE" for="edge" attr.name="label" attr.type="string"/>
  <key id="n_name" for="node" attr.name="name" attr.type="string"/>
  <key id="n_age" for="node" attr.name="age" attr.type="long"/>
  <graph id="G" edgedefault="directed">
    <node id="1">
      <data key="label">Person</data>
      <data key="n_name">Alice</data>
      <data key="n_age">30</data>
    </node>
    <node id="2">
      <data key="label">Person</data>
      <data key="n_name">Bob</data>
    </node>
    <edge id="e0" source="1" target="2">
      <data key="labelE">KNOWS</data>
    </edge>
  </graph>
</graphml>"#;

        let mut graph = Graph::new();
        let stats = GraphMlImporter::import(&mut graph, graphml.as_bytes()).unwrap();

        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(stats.edges_imported, 1);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    /// Properties, labels and escaped text survive import (and a roundtrip).
    #[test]
    fn test_import_graphml_properties_and_escapes() {
        let graphml = r#"<?xml version="1.0" encoding="UTF-8"?>
<graphml xmlns="http://graphml.graphdrawing.org/xmlns">
  <key id="label" for="node" attr.name="label" attr.type="string"/>
  <key id="labelE" for="edge" attr.name="label" attr.type="string"/>
  <key id="n_name" for="node" attr.name="name" attr.type="string"/>
  <key id="n_age" for="node" attr.name="age" attr.type="long"/>
  <key id="e_w" for="edge" attr.name="weight" attr.type="double"/>
  <graph id="G" edgedefault="directed">
    <node id="a&amp;1">
      <data key="label">Person</data>
      <data key="n_name">Tom &amp; Jerry &lt;3 &#x41;</data>
      <data key="n_age">30</data>
    </node>
    <node id="2"/>
    <edge id="e0" source="a&amp;1" target="2">
      <data key="labelE">KNOWS</data>
      <data key="e_w">1.5</data>
    </edge>
  </graph>
</graphml>"#;

        let mut graph = Graph::new();
        let stats = GraphMlImporter::import(&mut graph, graphml.as_bytes()).unwrap();
        assert_eq!(stats.nodes_imported, 2);
        assert_eq!(stats.edges_imported, 1);

        let alice = graph
            .nodes()
            .find(|n| n.has_label("Person"))
            .expect("Person node");
        assert_eq!(
            alice.get_property("name"),
            Some(&PropertyValue::String("Tom & Jerry <3 A".to_string()))
        );
        assert_eq!(alice.get_property("age"), Some(&PropertyValue::Int(30)));
        assert_eq!(alice.get_property("label"), None);

        let edge = graph.edges().next().expect("edge");
        assert_eq!(edge.label, "KNOWS");
        assert_eq!(
            edge.get_property("weight"),
            Some(&PropertyValue::Float(1.5))
        );

        // Roundtrip keeps the escaped value.
        let xml = GraphMlExporter::export_to_string(&graph).unwrap();
        let mut graph2 = Graph::new();
        GraphMlImporter::import(&mut graph2, xml.as_bytes()).unwrap();
        assert!(graph2.nodes().any(|n| {
            n.get_property("name") == Some(&PropertyValue::String("Tom & Jerry <3 A".to_string()))
        }));
    }

    #[test]
    fn test_import_graphml_rejects_bad_input() {
        let mut graph = Graph::new();
        // Undefined entity.
        let undefined = r#"<graphml><key id="k" for="node" attr.name="name"/><graph><node id="1"><data key="k">&nope;</data></node></graph></graphml>"#;
        assert!(GraphMlImporter::import(&mut graph, undefined.as_bytes()).is_err());
        // Mismatched end tag.
        let mismatched = r#"<graphml><graph><node id="1"></edge></graph></graphml>"#;
        assert!(GraphMlImporter::import(&mut graph, mismatched.as_bytes()).is_err());
    }

    /// A start tag with many attributes is handled in linear time
    /// (quick-xml < 0.41 was quadratic here, RUSTSEC-2026-0194).
    #[test]
    fn test_import_graphml_many_attributes() {
        let attrs: String = (0..50_000).map(|i| format!(" a{i}=\"x\"")).collect();
        let graphml = format!(r#"<graphml><graph><node id="1"{attrs}/></graph></graphml>"#);
        let started = std::time::Instant::now();
        let mut graph = Graph::new();
        let stats = GraphMlImporter::import(&mut graph, graphml.as_bytes()).unwrap();
        assert_eq!(stats.nodes_imported, 1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn test_graphml_roundtrip() {
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

        // Export
        let mut xml_output = Vec::new();
        GraphMlExporter::export(&graph1, &mut xml_output).unwrap();

        // Import
        let mut graph2 = Graph::new();
        GraphMlImporter::import(&mut graph2, xml_output.as_slice()).unwrap();

        assert_eq!(graph2.node_count(), 2);
        assert_eq!(graph2.edge_count(), 1);
    }
}
