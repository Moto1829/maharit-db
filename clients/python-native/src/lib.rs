//! Native Python extension for MaharitDB.
//!
//! Provides in-process graph operations without the TCP server overhead.
//!
//! # Usage (from Python)
//!
//! ```python
//! from maharit_native import Graph
//!
//! g = Graph()
//! alice = g.create_node("Person")
//! bob   = g.create_node("Person")
//! g.set_node_property(alice, "name", "Alice")
//! edge  = g.create_edge(alice, bob, "KNOWS")
//!
//! print(g.node_count())   # 2
//! print(g.get_node(alice))
//! # {'id': 0, 'labels': ['Person'], 'properties': {'name': 'Alice'}}
//! ```

use maharit_core::ConcurrentGraph;
use maharit_core::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// In-process graph database backed by lock-free DashMap shards.
#[pyclass(name = "Graph")]
struct PyGraph {
    inner: ConcurrentGraph,
}

#[pymethods]
impl PyGraph {
    /// Create an empty graph.
    #[new]
    fn new() -> Self {
        Self {
            inner: ConcurrentGraph::new(),
        }
    }

    /// Create a node with a single label. Returns the node ID.
    fn create_node(&self, label: &str) -> u64 {
        self.inner.create_node(label)
    }

    /// Create a node with multiple labels. Returns the node ID.
    fn create_node_with_labels(&self, labels: Vec<String>) -> u64 {
        self.inner.create_node_with_labels(labels)
    }

    /// Create a directed edge. Returns the edge ID.
    ///
    /// Raises `ValueError` if either node does not exist.
    fn create_edge(&self, from: u64, to: u64, label: &str) -> PyResult<u64> {
        self.inner
            .create_edge(from, to, label)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Delete a node and all its incident edges. Returns `True` if found.
    fn delete_node(&self, id: u64) -> bool {
        self.inner.delete_node(id).is_some()
    }

    /// Delete an edge by ID. Returns `True` if found.
    fn delete_edge(&self, id: u64) -> bool {
        self.inner.delete_edge(id).is_some()
    }

    /// Set a string property on a node.
    fn set_node_property(&self, node_id: u64, key: &str, value: &str) {
        self.inner.set_node_property(
            node_id,
            key,
            PropertyValue::String(value.to_string()),
        );
    }

    /// Set an integer property on a node.
    fn set_node_property_int(&self, node_id: u64, key: &str, value: i64) {
        self.inner
            .set_node_property(node_id, key, PropertyValue::Int(value));
    }

    /// Set a float property on a node.
    fn set_node_property_float(&self, node_id: u64, key: &str, value: f64) {
        self.inner
            .set_node_property(node_id, key, PropertyValue::Float(value));
    }

    /// Set a boolean property on a node.
    fn set_node_property_bool(&self, node_id: u64, key: &str, value: bool) {
        self.inner
            .set_node_property(node_id, key, PropertyValue::Bool(value));
    }

    /// Set a string property on an edge.
    fn set_edge_property(&self, edge_id: u64, key: &str, value: &str) {
        self.inner.set_edge_property(
            edge_id,
            key,
            PropertyValue::String(value.to_string()),
        );
    }

    /// Return a dict representation of the node, or `None` if not found.
    fn get_node(&self, py: Python<'_>, id: u64) -> Option<PyObject> {
        self.inner.with_node(id, |node| {
            let d = PyDict::new_bound(py);
            d.set_item("id", node.id).unwrap();
            let labels: Vec<&str> = node.labels.iter().map(|s| s.as_str()).collect();
            d.set_item("labels", PyList::new_bound(py, &labels)).unwrap();
            let props = PyDict::new_bound(py);
            for (k, v) in node.properties.iter() {
                props.set_item(k, pv_to_py(py, v)).unwrap();
            }
            d.set_item("properties", props).unwrap();
            d.into()
        })
    }

    /// Return a dict representation of the edge, or `None` if not found.
    fn get_edge(&self, py: Python<'_>, id: u64) -> Option<PyObject> {
        self.inner.with_edge(id, |edge| {
            let d = PyDict::new_bound(py);
            d.set_item("id", edge.id).unwrap();
            d.set_item("label", edge.label.as_str()).unwrap();
            d.set_item("from", edge.from).unwrap();
            d.set_item("to", edge.to).unwrap();
            let props = PyDict::new_bound(py);
            for (k, v) in edge.properties.iter() {
                props.set_item(k, pv_to_py(py, v)).unwrap();
            }
            d.set_item("properties", props).unwrap();
            d.into()
        })
    }

    /// Outgoing edge IDs for a node.
    fn get_outgoing_edges(&self, node_id: u64) -> Vec<u64> {
        self.inner.get_outgoing_edges(node_id)
    }

    /// Incoming edge IDs for a node.
    fn get_incoming_edges(&self, node_id: u64) -> Vec<u64> {
        self.inner.get_incoming_edges(node_id)
    }

    /// Number of nodes.
    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Number of edges.
    fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }
}

/// Convert a `PropertyValue` to a Python object.
fn pv_to_py(py: Python<'_>, pv: &PropertyValue) -> PyObject {
    use pyo3::ToPyObject;
    match pv {
        PropertyValue::Null => py.None(),
        PropertyValue::Bool(b) => b.to_object(py),
        PropertyValue::Int(n) => n.to_object(py),
        PropertyValue::Float(f) => f.to_object(py),
        PropertyValue::String(s) => s.to_object(py),
        other => format!("{:?}", other).to_object(py),
    }
}

/// Register the Python module.
#[pymodule]
fn maharit_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyGraph>()?;
    Ok(())
}
