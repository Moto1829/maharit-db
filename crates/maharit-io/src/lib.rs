pub mod bulk_io;
pub mod csv_io;
pub mod graphml_io;
pub mod json_io;

pub use bulk_io::{BulkConfig, BulkLoader, BulkProgress, BulkStats};
pub use csv_io::{CsvExporter, CsvImporter};
pub use graphml_io::{GraphMlExporter, GraphMlImporter};
pub use json_io::{JsonExporter, JsonImporter};

use thiserror::Error;

/// インポート/エクスポートのエラー型
#[derive(Debug, Error)]
pub enum IoError {
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid format: {0}")]
    InvalidFormat(String),

    #[error("graph error: {0}")]
    Graph(#[from] maharit_core::GraphError),
}

pub type Result<T> = std::result::Result<T, IoError>;
