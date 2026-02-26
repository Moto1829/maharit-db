//! GraphML format import/export
//!
//! GraphML is an XML-based file format for graphs.
//! See: http://graphml.graphdrawing.org/

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};

use maharit_core::{Graph, PropertyValue};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use crate::{IoError, Result};

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

/// GraphML importer
pub struct GraphMlImporter;

impl GraphMlImporter {
    /// Import a graph from GraphML format
    pub fn import<R: Read>(graph: &mut Graph, reader: R) -> Result<ImportStats> {
        let buf_reader = BufReader::new(reader);
        let mut xml_reader = Reader::from_reader(buf_reader);
        xml_reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut keys: HashMap<String, KeyDef> = HashMap::new();
        let mut id_map: HashMap<String, u64> = HashMap::new();

        let mut nodes_imported = 0;
        let mut edges_imported = 0;

        // Current element state
        let mut current_node_id: Option<String> = None;
        let mut current_edge: Option<(String, String)> = None;
        let mut current_data_key: Option<String> = None;
        let mut current_node_properties: HashMap<String, PropertyValue> = HashMap::new();
        let mut current_edge_properties: HashMap<String, PropertyValue> = HashMap::new();
        let mut current_node_label: Option<String> = None;
        let mut current_edge_label: Option<String> = None;

        loop {
            match xml_reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    match e.name().as_ref() {
                        b"key" => {
                            // Parse key definition
                            if let Some(key_def) = Self::parse_key(e) {
                                keys.insert(key_def.id.clone(), key_def);
                            }
                        }
                        b"node" => {
                            // Start of node
                            current_node_id = Self::get_attr(e, b"id");
                            current_node_properties.clear();
                            current_node_label = None;
                        }
                        b"edge" => {
                            // Start of edge
                            let source = Self::get_attr(e, b"source");
                            let target = Self::get_attr(e, b"target");
                            if let (Some(s), Some(t)) = (source, target) {
                                current_edge = Some((s, t));
                            }
                            current_edge_properties.clear();
                            current_edge_label = None;
                        }
                        b"data" => {
                            current_data_key = Self::get_attr(e, b"key");
                        }
                        _ => {}
                    }

                    // Handle empty elements (self-closing tags)
                    if matches!(xml_reader.read_event_into(&mut buf), Ok(Event::Empty(_))) {
                        // Process empty node
                        if let Some(ref node_id) = current_node_id {
                            let label = current_node_label
                                .clone()
                                .unwrap_or_else(|| "Node".to_string());
                            let internal_id = graph.create_node(&label);
                            id_map.insert(node_id.clone(), internal_id);
                            nodes_imported += 1;
                            current_node_id = None;
                        }
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if let Some(ref key_id) = current_data_key {
                        let text = e.unescape().unwrap_or_default().to_string();

                        // Check if this is a label key
                        if let Some(key_def) = keys.get(key_id) {
                            if key_def.name == "label"
                                || key_def.name == "labelV"
                                || key_def.name == "labelE"
                            {
                                if current_node_id.is_some() {
                                    current_node_label = Some(text.clone());
                                } else if current_edge.is_some() {
                                    current_edge_label = Some(text.clone());
                                }
                            }
                        }

                        let value = Self::parse_value(&text, keys.get(key_id));

                        if current_node_id.is_some() {
                            let prop_name = keys
                                .get(key_id)
                                .map(|k| k.name.clone())
                                .unwrap_or_else(|| key_id.clone());
                            current_node_properties.insert(prop_name, value);
                        } else if current_edge.is_some() {
                            let prop_name = keys
                                .get(key_id)
                                .map(|k| k.name.clone())
                                .unwrap_or_else(|| key_id.clone());
                            current_edge_properties.insert(prop_name, value);
                        }
                    }
                }
                Ok(Event::End(ref e)) => match e.name().as_ref() {
                    b"node" => {
                        if let Some(ref node_id) = current_node_id {
                            let label = current_node_label
                                .take()
                                .unwrap_or_else(|| "Node".to_string());
                            let internal_id = graph.create_node(&label);
                            id_map.insert(node_id.clone(), internal_id);

                            if let Some(node) = graph.get_node_mut(internal_id) {
                                for (k, v) in current_node_properties.drain() {
                                    if k != "label" && k != "labelV" {
                                        node.set_property(k, v);
                                    }
                                }
                            }
                            nodes_imported += 1;
                        }
                        current_node_id = None;
                    }
                    b"edge" => {
                        if let Some((source, target)) = current_edge.take() {
                            let from_id = id_map.get(&source).copied();
                            let to_id = id_map.get(&target).copied();

                            if let (Some(from), Some(to)) = (from_id, to_id) {
                                let label = current_edge_label
                                    .take()
                                    .unwrap_or_else(|| "EDGE".to_string());
                                if let Ok(edge_id) = graph.create_edge(from, to, &label) {
                                    if let Some(edge) = graph.get_edge_mut(edge_id) {
                                        for (k, v) in current_edge_properties.drain() {
                                            if k != "label" && k != "labelE" {
                                                edge.set_property(k, v);
                                            }
                                        }
                                    }
                                    edges_imported += 1;
                                }
                            }
                        }
                    }
                    b"data" => {
                        current_data_key = None;
                    }
                    _ => {}
                },
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(IoError::InvalidFormat(format!("XML parse error: {}", e)));
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(ImportStats {
            nodes_imported,
            edges_imported,
        })
    }

    fn parse_key(e: &BytesStart) -> Option<KeyDef> {
        let id = Self::get_attr(e, b"id")?;
        let name = Self::get_attr(e, b"attr.name").unwrap_or_else(|| id.clone());
        let for_str = Self::get_attr(e, b"for").unwrap_or_else(|| "all".to_string());
        let attr_type_str = Self::get_attr(e, b"attr.type").unwrap_or_else(|| "string".to_string());

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

    fn get_attr(e: &BytesStart, name: &[u8]) -> Option<String> {
        e.attributes()
            .filter_map(|a| a.ok())
            .find(|a| a.key.as_ref() == name)
            .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
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

            // Label
            Self::write_data(&mut xml_writer, "label", &node.label)?;

            // Properties
            for (key, value) in &node.properties {
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
            for (key, value) in &edge.properties {
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

    fn collect_keys(graph: &Graph) -> (Vec<(String, &'static str)>, Vec<(String, &'static str)>) {
        let mut node_keys: HashMap<String, &'static str> = HashMap::new();
        let mut edge_keys: HashMap<String, &'static str> = HashMap::new();

        for node in graph.nodes() {
            for (key, value) in &node.properties {
                node_keys
                    .entry(key.clone())
                    .or_insert_with(|| Self::property_type(value));
            }
        }

        for edge in graph.edges() {
            for (key, value) in &edge.properties {
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
