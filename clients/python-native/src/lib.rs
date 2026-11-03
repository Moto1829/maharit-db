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
    fn get_node<'py>(&self, py: Python<'py>, id: u64) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.inner
            .with_node(id, |node| {
                let d = PyDict::new(py);
                d.set_item("id", node.id)?;
                d.set_item("labels", PyList::new(py, &node.labels)?)?;
                d.set_item("properties", props_to_py(py, node.properties.iter())?)?;
                Ok(d)
            })
            .transpose()
    }

    /// Return a dict representation of the edge, or `None` if not found.
    fn get_edge<'py>(&self, py: Python<'py>, id: u64) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.inner
            .with_edge(id, |edge| {
                let d = PyDict::new(py);
                d.set_item("id", edge.id)?;
                d.set_item("label", edge.label.as_str())?;
                d.set_item("from", edge.from)?;
                d.set_item("to", edge.to)?;
                d.set_item("properties", props_to_py(py, edge.properties.iter())?)?;
                Ok(d)
            })
            .transpose()
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

/// Build a Python dict from property key/value pairs.
fn props_to_py<'py, 'a>(
    py: Python<'py>,
    props: impl Iterator<Item = (&'a String, &'a PropertyValue)>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in props {
        d.set_item(k, pv_to_py(py, v)?)?;
    }
    Ok(d)
}

/// Convert a `PropertyValue` to a Python object.
fn pv_to_py<'py>(py: Python<'py>, pv: &PropertyValue) -> PyResult<Bound<'py, PyAny>> {
    Ok(match pv {
        PropertyValue::Null => py.None().into_bound(py),
        PropertyValue::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
        PropertyValue::Int(n) => n.into_pyobject(py)?.into_any(),
        PropertyValue::Float(f) => f.into_pyobject(py)?.into_any(),
        PropertyValue::String(s) => s.into_pyobject(py)?.into_any(),
        other => format!("{:?}", other).into_pyobject(py)?.into_any(),
    })
}

/// Register the Python module.
#[pymodule]
fn maharit_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyGraph>()?;
    Ok(())
}
