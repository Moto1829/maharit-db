//! Bulk loading functionality for efficient large data import
//!
//! Provides progress tracking and optimized loading for large datasets.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use maharit_core::{Graph, PropertyValue};

use crate::{IoError, Result};

/// Progress callback type
pub type ProgressCallback = Box<dyn Fn(BulkProgress) + Send>;

/// Progress information during bulk loading
#[derive(Debug, Clone)]
pub struct BulkProgress {
    /// Current phase (e.g., "Loading nodes", "Loading edges")
    pub phase: String,
    /// Number of items processed so far
    pub processed: usize,
    /// Total number of items (if known)
    pub total: Option<usize>,
    /// Percentage complete (0.0 - 100.0)
    pub percentage: Option<f64>,
}

impl BulkProgress {
    fn new(phase: &str, processed: usize, total: Option<usize>) -> Self {
        let percentage = total.map(|t| {
            if t > 0 {
                (processed as f64 / t as f64) * 100.0
            } else {
                100.0
            }
        });
        Self {
            phase: phase.to_string(),
            processed,
            total,
            percentage,
        }
    }
}

/// Bulk loading statistics
#[derive(Debug, Clone, Default)]
pub struct BulkStats {
    /// Number of nodes imported
    pub nodes_imported: usize,
    /// Number of edges imported
    pub edges_imported: usize,
    /// Number of errors encountered
    pub errors: usize,
    /// Error messages (limited to first 100)
    pub error_messages: Vec<String>,
}

impl BulkStats {
    fn add_error(&mut self, msg: String) {
        self.errors += 1;
        if self.error_messages.len() < 100 {
            self.error_messages.push(msg);
        }
    }
}

/// Configuration for bulk loading
#[derive(Debug, Clone)]
pub struct BulkConfig {
    /// Batch size for processing (how often to report progress)
    pub batch_size: usize,
    /// Skip rows with errors instead of failing
    pub skip_errors: bool,
    /// Maximum number of errors before aborting
    pub max_errors: usize,
}

impl Default for BulkConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            skip_errors: true,
            max_errors: 1000,
        }
    }
}

/// Bulk loader for large datasets
pub struct BulkLoader {
    config: BulkConfig,
    progress_callback: Option<ProgressCallback>,
}

impl BulkLoader {
    /// Create a new bulk loader with default configuration
    pub fn new() -> Self {
        Self {
            config: BulkConfig::default(),
            progress_callback: None,
        }
    }

    /// Create a new bulk loader with custom configuration
    pub fn with_config(config: BulkConfig) -> Self {
        Self {
            config,
            progress_callback: None,
        }
    }

    /// Set a progress callback
    pub fn on_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(BulkProgress) + Send + 'static,
    {
        self.progress_callback = Some(Box::new(callback));
        self
    }

    /// Report progress
    fn report_progress(&self, phase: &str, processed: usize, total: Option<usize>) {
        if let Some(ref callback) = self.progress_callback {
            callback(BulkProgress::new(phase, processed, total));
        }
    }

    /// Load nodes from a CSV file
    pub fn load_nodes_from_file<P: AsRef<Path>>(
        &self,
        graph: &mut Graph,
        path: P,
    ) -> Result<(BulkStats, HashMap<String, u64>)> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let total_lines = Self::count_lines(&file)?;
        let file = File::open(path)?; // Reopen after counting
        self.load_nodes(graph, file, Some(total_lines.saturating_sub(1)))
    }

    /// Load nodes from a reader
    pub fn load_nodes<R: Read>(
        &self,
        graph: &mut Graph,
        reader: R,
        total: Option<usize>,
    ) -> Result<(BulkStats, HashMap<String, u64>)> {
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

        let mut stats = BulkStats::default();
        let mut id_map: HashMap<String, u64> = HashMap::new();

        self.report_progress("Loading nodes", 0, total);

        for (idx, result) in csv_reader.records().enumerate() {
            if stats.errors >= self.config.max_errors {
                break;
            }

            match result {
                Ok(record) => {
                    let external_id = record.get(0).unwrap_or("").to_string();
                    let label = record.get(1).unwrap_or("").to_string();

                    let node_id = graph.create_node(&label);
                    id_map.insert(external_id, node_id);

                    if let Some(node) = graph.get_node_mut(node_id) {
                        for (i, header) in headers.iter().enumerate().skip(2) {
                            if let Some(value) = record.get(i) {
                                if !value.is_empty() {
                                    let prop_value = Self::parse_value(value);
                                    node.set_property(header.clone(), prop_value);
                                }
                            }
                        }
                    }

                    stats.nodes_imported += 1;
                }
                Err(e) => {
                    if self.config.skip_errors {
                        stats.add_error(format!("Row {}: {}", idx + 1, e));
                    } else {
                        return Err(e.into());
                    }
                }
            }

            // Report progress periodically
            if (idx + 1) % self.config.batch_size == 0 {
                self.report_progress("Loading nodes", idx + 1, total);
            }
        }

        self.report_progress("Loading nodes", stats.nodes_imported, total);

        Ok((stats, id_map))
    }

    /// Load edges from a CSV file
    pub fn load_edges_from_file<P: AsRef<Path>>(
        &self,
        graph: &mut Graph,
        path: P,
        id_map: &HashMap<String, u64>,
    ) -> Result<BulkStats> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let total_lines = Self::count_lines(&file)?;
        let file = File::open(path)?;
        self.load_edges(graph, file, id_map, Some(total_lines.saturating_sub(1)))
    }

    /// Load edges from a reader
    pub fn load_edges<R: Read>(
        &self,
        graph: &mut Graph,
        reader: R,
        id_map: &HashMap<String, u64>,
        total: Option<usize>,
    ) -> Result<BulkStats> {
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

        let mut stats = BulkStats::default();

        self.report_progress("Loading edges", 0, total);

        for (idx, result) in csv_reader.records().enumerate() {
            if stats.errors >= self.config.max_errors {
                break;
            }

            match result {
                Ok(record) => {
                    let from_external = record.get(0).unwrap_or("");
                    let to_external = record.get(1).unwrap_or("");
                    let edge_type = record.get(2).unwrap_or("").to_string();

                    let from_id = id_map.get(from_external).copied();
                    let to_id = id_map.get(to_external).copied();

                    match (from_id, to_id) {
                        (Some(from), Some(to)) => match graph.create_edge(from, to, &edge_type) {
                            Ok(edge_id) => {
                                if let Some(edge) = graph.get_edge_mut(edge_id) {
                                    for (i, header) in headers.iter().enumerate().skip(3) {
                                        if let Some(value) = record.get(i) {
                                            if !value.is_empty() {
                                                let prop_value = Self::parse_value(value);
                                                edge.set_property(header.clone(), prop_value);
                                            }
                                        }
                                    }
                                }
                                stats.edges_imported += 1;
                            }
                            Err(e) => {
                                if self.config.skip_errors {
                                    stats.add_error(format!("Row {}: {}", idx + 1, e));
                                } else {
                                    return Err(e.into());
                                }
                            }
                        },
                        _ => {
                            if self.config.skip_errors {
                                stats.add_error(format!(
                                    "Row {}: Unknown node ID (from={}, to={})",
                                    idx + 1,
                                    from_external,
                                    to_external
                                ));
                            } else {
                                return Err(IoError::InvalidFormat(format!(
                                    "Unknown node ID: from={}, to={}",
                                    from_external, to_external
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    if self.config.skip_errors {
                        stats.add_error(format!("Row {}: {}", idx + 1, e));
                    } else {
                        return Err(e.into());
                    }
                }
            }

            if (idx + 1) % self.config.batch_size == 0 {
                self.report_progress("Loading edges", idx + 1, total);
            }
        }

        self.report_progress("Loading edges", stats.edges_imported, total);

        Ok(stats)
    }

    /// Count lines in a file (for progress calculation)
    fn count_lines<R: Read>(reader: R) -> Result<usize> {
        let buf_reader = BufReader::new(reader);
        Ok(buf_reader.lines().count())
    }

    /// Parse a string value to PropertyValue
    fn parse_value(s: &str) -> PropertyValue {
        if let Ok(n) = s.parse::<i64>() {
            return PropertyValue::Int(n);
        }
        if let Ok(n) = s.parse::<f64>() {
            return PropertyValue::Float(n);
        }
        match s.to_lowercase().as_str() {
            "true" => return PropertyValue::Bool(true),
            "false" => return PropertyValue::Bool(false),
            "null" | "" => return PropertyValue::Null,
            _ => {}
        }
        PropertyValue::String(s.to_string())
    }
}

impl Default for BulkLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bulk_load_nodes() {
        let csv_data = "id,label,name,age\n1,Person,Alice,30\n2,Person,Bob,25\n3,Person,Carol,35\n";
        let mut graph = Graph::new();

        let loader = BulkLoader::new();
        let (stats, id_map) = loader
            .load_nodes(&mut graph, csv_data.as_bytes(), Some(3))
            .unwrap();

        assert_eq!(stats.nodes_imported, 3);
        assert_eq!(stats.errors, 0);
        assert_eq!(id_map.len(), 3);
        assert_eq!(graph.node_count(), 3);
    }

    #[test]
    fn test_bulk_load_edges() {
        let nodes_csv = "id,label,name\n1,Person,Alice\n2,Person,Bob\n3,Person,Carol\n";
        let edges_csv = "from,to,type,since\n1,2,KNOWS,2020\n2,3,KNOWS,2021\n";

        let mut graph = Graph::new();
        let loader = BulkLoader::new();

        let (_, id_map) = loader
            .load_nodes(&mut graph, nodes_csv.as_bytes(), None)
            .unwrap();

        let stats = loader
            .load_edges(&mut graph, edges_csv.as_bytes(), &id_map, Some(2))
            .unwrap();

        assert_eq!(stats.edges_imported, 2);
        assert_eq!(stats.errors, 0);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn test_bulk_load_with_errors() {
        let nodes_csv = "id,label,name\n1,Person,Alice\n2,Person,Bob\n";
        let edges_csv = "from,to,type\n1,2,KNOWS\n1,99,INVALID\n2,1,KNOWS\n";

        let mut graph = Graph::new();
        let config = BulkConfig {
            skip_errors: true,
            ..Default::default()
        };
        let loader = BulkLoader::with_config(config);

        let (_, id_map) = loader
            .load_nodes(&mut graph, nodes_csv.as_bytes(), None)
            .unwrap();

        let stats = loader
            .load_edges(&mut graph, edges_csv.as_bytes(), &id_map, None)
            .unwrap();

        assert_eq!(stats.edges_imported, 2);
        assert_eq!(stats.errors, 1);
        assert!(!stats.error_messages.is_empty());
    }

    #[test]
    fn test_progress_callback() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let nodes_csv = "id,label\n1,A\n2,B\n3,C\n4,D\n5,E\n";
        let mut graph = Graph::new();

        let progress_count = Arc::new(AtomicUsize::new(0));
        let progress_count_clone = progress_count.clone();

        let config = BulkConfig {
            batch_size: 2,
            ..Default::default()
        };

        let loader = BulkLoader::with_config(config).on_progress(move |_progress| {
            progress_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        loader
            .load_nodes(&mut graph, nodes_csv.as_bytes(), Some(5))
            .unwrap();

        // Should have called progress multiple times
        assert!(progress_count.load(Ordering::SeqCst) > 0);
    }
}
