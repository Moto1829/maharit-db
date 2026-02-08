//! Structured logging for the database server.
//!
//! Provides JSON-formatted log output with configurable log levels.
//! This is a lightweight, zero-dependency (beyond std) structured logger.

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Level {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    /// Parse a log level from string (case-insensitive).
    pub fn from_str_ci(s: &str) -> Option<Level> {
        match s.to_ascii_uppercase().as_str() {
            "TRACE" => Some(Level::Trace),
            "DEBUG" => Some(Level::Debug),
            "INFO" => Some(Level::Info),
            "WARN" | "WARNING" => Some(Level::Warn),
            "ERROR" => Some(Level::Error),
            _ => None,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Global log level filter.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Set the global log level filter.
pub fn set_level(level: Level) {
    LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

/// Get the current global log level filter.
pub fn get_level() -> Level {
    match LOG_LEVEL.load(Ordering::SeqCst) {
        0 => Level::Trace,
        1 => Level::Debug,
        2 => Level::Info,
        3 => Level::Warn,
        _ => Level::Error,
    }
}

/// Check if a given level is enabled under the current filter.
pub fn is_enabled(level: Level) -> bool {
    level as u8 >= LOG_LEVEL.load(Ordering::SeqCst)
}

/// A structured log entry with contextual fields.
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub module: Option<String>,
    pub fields: Vec<(String, String)>,
}

impl LogEntry {
    /// Create a new log entry.
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
            module: None,
            fields: Vec::new(),
        }
    }

    /// Add a module path.
    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }

    /// Add a contextual field.
    pub fn field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((key.into(), value.into()));
        self
    }

    /// Format as JSON string.
    pub fn to_json(&self) -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let mut json = format!(
            "{{\"timestamp\":{},\"level\":\"{}\"",
            timestamp,
            self.level.as_str()
        );

        if let Some(ref module) = self.module {
            json.push_str(&format!(",\"module\":\"{}\"", escape_json(module)));
        }

        json.push_str(&format!(",\"message\":\"{}\"", escape_json(&self.message)));

        for (key, value) in &self.fields {
            json.push_str(&format!(
                ",\"{}\":\"{}\"",
                escape_json(key),
                escape_json(value)
            ));
        }

        json.push('}');
        json
    }
}

/// Escape a string for JSON output.
fn escape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

/// Write a log entry to stderr if the level is enabled.
pub fn log(entry: LogEntry) {
    if !is_enabled(entry.level) {
        return;
    }
    let json = entry.to_json();
    let _ = writeln!(std::io::stderr(), "{}", json);
}

/// Convenience function: log at INFO level.
pub fn info(message: impl Into<String>) {
    log(LogEntry::new(Level::Info, message));
}

/// Convenience function: log at WARN level.
pub fn warn(message: impl Into<String>) {
    log(LogEntry::new(Level::Warn, message));
}

/// Convenience function: log at ERROR level.
pub fn error(message: impl Into<String>) {
    log(LogEntry::new(Level::Error, message));
}

/// Convenience function: log at DEBUG level.
pub fn debug(message: impl Into<String>) {
    log(LogEntry::new(Level::Debug, message));
}

/// Convenience function: log at TRACE level.
pub fn trace(message: impl Into<String>) {
    log(LogEntry::new(Level::Trace, message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_ordering() {
        assert!(Level::Trace < Level::Debug);
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
    }

    #[test]
    fn test_level_display() {
        assert_eq!(Level::Trace.to_string(), "TRACE");
        assert_eq!(Level::Debug.to_string(), "DEBUG");
        assert_eq!(Level::Info.to_string(), "INFO");
        assert_eq!(Level::Warn.to_string(), "WARN");
        assert_eq!(Level::Error.to_string(), "ERROR");
    }

    #[test]
    fn test_level_from_str() {
        assert_eq!(Level::from_str_ci("trace"), Some(Level::Trace));
        assert_eq!(Level::from_str_ci("DEBUG"), Some(Level::Debug));
        assert_eq!(Level::from_str_ci("Info"), Some(Level::Info));
        assert_eq!(Level::from_str_ci("WARN"), Some(Level::Warn));
        assert_eq!(Level::from_str_ci("WARNING"), Some(Level::Warn));
        assert_eq!(Level::from_str_ci("error"), Some(Level::Error));
        assert_eq!(Level::from_str_ci("invalid"), None);
    }

    #[test]
    fn test_log_entry_to_json() {
        let entry = LogEntry::new(Level::Info, "Server started")
            .module("maharit_server")
            .field("port", "7687");

        let json = entry.to_json();
        assert!(json.contains("\"level\":\"INFO\""));
        assert!(json.contains("\"message\":\"Server started\""));
        assert!(json.contains("\"module\":\"maharit_server\""));
        assert!(json.contains("\"port\":\"7687\""));
        assert!(json.contains("\"timestamp\":"));
    }

    #[test]
    fn test_log_entry_without_module() {
        let entry = LogEntry::new(Level::Error, "Something failed");
        let json = entry.to_json();
        assert!(json.contains("\"level\":\"ERROR\""));
        assert!(json.contains("\"message\":\"Something failed\""));
        assert!(!json.contains("\"module\""));
    }

    #[test]
    fn test_log_entry_with_multiple_fields() {
        let entry = LogEntry::new(Level::Debug, "Query executed")
            .field("query", "MATCH (n) RETURN n")
            .field("duration_ms", "42")
            .field("rows", "10");

        let json = entry.to_json();
        assert!(json.contains("\"query\":\"MATCH (n) RETURN n\""));
        assert!(json.contains("\"duration_ms\":\"42\""));
        assert!(json.contains("\"rows\":\"10\""));
    }

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_json("path\\to\\file"), "path\\\\to\\\\file");
        assert_eq!(escape_json("tab\there"), "tab\\there");
    }

    #[test]
    fn test_is_enabled() {
        set_level(Level::Info);
        assert!(!is_enabled(Level::Trace));
        assert!(!is_enabled(Level::Debug));
        assert!(is_enabled(Level::Info));
        assert!(is_enabled(Level::Warn));
        assert!(is_enabled(Level::Error));

        set_level(Level::Trace);
        assert!(is_enabled(Level::Trace));
        assert!(is_enabled(Level::Debug));

        set_level(Level::Error);
        assert!(!is_enabled(Level::Warn));
        assert!(is_enabled(Level::Error));

        // Reset to Info for other tests
        set_level(Level::Info);
    }

    #[test]
    fn test_get_set_level() {
        set_level(Level::Debug);
        assert_eq!(get_level(), Level::Debug);

        set_level(Level::Error);
        assert_eq!(get_level(), Level::Error);

        // Reset
        set_level(Level::Info);
        assert_eq!(get_level(), Level::Info);
    }

    #[test]
    fn test_json_output_is_valid() {
        let entry = LogEntry::new(Level::Info, "test message")
            .module("test")
            .field("key", "value");

        let json = entry.to_json();
        // Verify it's valid JSON by checking structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));

        // Parse with serde_json to validate
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["message"], "test message");
        assert_eq!(parsed["module"], "test");
        assert_eq!(parsed["key"], "value");
        assert!(parsed["timestamp"].is_number());
    }

    #[test]
    fn test_escape_special_characters_in_json() {
        let entry = LogEntry::new(Level::Error, "Error: \"bad input\"\nline2");
        let json = entry.to_json();

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["message"], "Error: \"bad input\"\nline2");
    }
}
