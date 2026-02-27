//! Structured logging for the database server.
//!
//! Provides JSON-formatted log output with configurable log levels.
//! This is a lightweight, zero-dependency (beyond std) structured logger.
//!
//! # Log rotation
//!
//! Use [`RotatingLogger`] to write structured JSON log entries to a file with
//! automatic size-based rotation.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, atomic::{AtomicU8, Ordering}};
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

// ---------------------------------------------------------------------------
// Rotating file logger
// ---------------------------------------------------------------------------

/// A structured JSON logger that writes to a file and rotates when the file
/// exceeds a configurable size threshold.
///
/// # Rotation scheme
///
/// When the active log file grows beyond `max_size_bytes` the logger shifts
/// existing rotated files up by one generation and renames the active file to
/// `<log_file>.1`:
///
/// ```text
/// maharit.log.4  → deleted (when max_generations == 5)
/// maharit.log.3  → maharit.log.4
/// maharit.log.2  → maharit.log.3
/// maharit.log.1  → maharit.log.2
/// maharit.log    → maharit.log.1
/// (new empty maharit.log created on the next write)
/// ```
///
/// # Thread safety
///
/// All public methods take `&self`.  A [`Mutex`] serialises concurrent writes
/// so the struct can safely be wrapped in an `Arc` and shared across threads.
///
/// # Example
///
/// ```no_run
/// use maharit_server::logging::RotatingLogger;
///
/// let logger = RotatingLogger::new("maharit.log", 100 * 1024 * 1024, 5);
/// logger.write_entry("{\"level\":\"INFO\",\"message\":\"server started\"}");
/// ```
#[derive(Debug)]
pub struct RotatingLogger {
    /// Path to the active log file.
    log_file: String,
    /// Rotate when the file exceeds this size in bytes.
    max_size_bytes: u64,
    /// Maximum number of rotated archives to keep.
    max_generations: u32,
    /// Mutex used to serialise concurrent writes.
    inner: Mutex<()>,
}

impl RotatingLogger {
    /// Default maximum file size: 100 MiB.
    pub const DEFAULT_MAX_SIZE: u64 = 100 * 1024 * 1024;
    /// Default number of rotated generations to keep.
    pub const DEFAULT_MAX_GENERATIONS: u32 = 5;

    /// Create a new [`RotatingLogger`].
    ///
    /// # Arguments
    ///
    /// * `log_file`        – path to the log file (created on first write).
    /// * `max_size_bytes`  – rotate when the file exceeds this size.
    /// * `max_generations` – maximum number of rotated archives to keep.
    pub fn new(
        log_file: impl Into<String>,
        max_size_bytes: u64,
        max_generations: u32,
    ) -> Self {
        Self {
            log_file: log_file.into(),
            max_size_bytes,
            max_generations,
            inner: Mutex::new(()),
        }
    }

    /// Create a [`RotatingLogger`] with the default limits:
    /// `max_size_bytes = 100 MiB` and `max_generations = 5`.
    pub fn with_defaults(log_file: impl Into<String>) -> Self {
        Self::new(log_file, Self::DEFAULT_MAX_SIZE, Self::DEFAULT_MAX_GENERATIONS)
    }

    /// Write a [`LogEntry`] to the file as a JSON line, rotating if necessary.
    ///
    /// The entry is formatted via [`LogEntry::to_json`] and appended with a
    /// trailing newline.
    pub fn write(&self, entry: &LogEntry) {
        let line = format!("{}\n", entry.to_json());
        self.append_line(&line);
    }

    /// Append a raw pre-formatted string (expected to be a single JSON line) to
    /// the log file, rotating first if the file exceeds `max_size_bytes`.
    pub fn write_entry(&self, line: &str) {
        let line = if line.ends_with('\n') {
            line.to_string()
        } else {
            format!("{line}\n")
        };
        self.append_line(&line);
    }

    /// Return the path to the active log file.
    pub fn log_file(&self) -> &str {
        &self.log_file
    }

    /// Return the configured maximum file size in bytes.
    pub fn max_size_bytes(&self) -> u64 {
        self.max_size_bytes
    }

    /// Return the configured maximum number of rotated generations.
    pub fn max_generations(&self) -> u32 {
        self.max_generations
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn append_line(&self, line: &str) {
        let _guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.rotate_if_needed();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// Rotate the log file if it currently exceeds `max_size_bytes`.
    fn rotate_if_needed(&self) {
        let current_size = match std::fs::metadata(&self.log_file) {
            Ok(m) => m.len(),
            Err(_) => return, // file does not exist yet
        };

        if current_size < self.max_size_bytes {
            return;
        }

        // Remove the oldest generation that would overflow the limit.
        let oldest = format!("{}.{}", self.log_file, self.max_generations);
        let _ = std::fs::remove_file(&oldest);

        // Shift each existing generation up by one.
        for generation in (1..self.max_generations).rev() {
            let from = format!("{}.{}", self.log_file, generation);
            let to = format!("{}.{}", self.log_file, generation + 1);
            let _ = std::fs::rename(&from, &to);
        }

        // Rename the active file to .1
        let _ = std::fs::rename(&self.log_file, format!("{}.1", self.log_file));

        // Create a fresh, empty active file for the next append.
        let _ = File::create(&self.log_file);
    }
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

    // -----------------------------------------------------------------------
    // RotatingLogger tests
    // -----------------------------------------------------------------------

    use std::fs;
    use std::path::PathBuf;

    /// Return the path to a log file in a per-test temporary directory.
    ///
    /// The directory is NOT created here; call `setup_log_dir(path)` after
    /// any cleanup to ensure the directory exists before the logger runs.
    fn tmp_log_path(test_name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("maharit_log_test_{}", test_name));
        dir.join("maharit.log")
    }

    /// Remove all log files for a test and recreate the parent directory.
    fn setup_log_dir(path: &PathBuf, max_generations: u32) {
        if let Some(parent) = path.parent() {
            // Remove existing log files.
            let _ = fs::remove_file(path);
            for i in 1..=max_generations {
                let _ = fs::remove_file(format!("{}.{}", path.display(), i));
            }
            // Ensure the parent directory exists.
            fs::create_dir_all(parent).expect("could not create temp test dir");
        }
    }

    fn cleanup_log(path: &PathBuf, max_generations: u32) {
        let _ = fs::remove_file(path);
        for i in 1..=max_generations {
            let _ = fs::remove_file(format!("{}.{}", path.display(), i));
        }
        // Best-effort remove of the parent dir (only succeeds when empty).
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    #[test]
    fn test_rotating_logger_creates_file_on_first_write() {
        let path = tmp_log_path("creates_file");
        setup_log_dir(&path, 5);

        let logger = RotatingLogger::new(path.to_str().unwrap(), 1024, 3);
        logger.write_entry("{\"message\":\"hello\"}");

        assert!(path.exists(), "log file should be created on first write");
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"message\":\"hello\""));

        cleanup_log(&path, 3);
    }

    #[test]
    fn test_rotating_logger_write_entry_appends_newline() {
        let path = tmp_log_path("newline");
        setup_log_dir(&path, 5);

        let logger = RotatingLogger::new(path.to_str().unwrap(), 1024, 3);
        logger.write_entry("{\"a\":1}");
        logger.write_entry("{\"b\":2}");

        let contents = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "each entry should be on its own line");

        cleanup_log(&path, 3);
    }

    #[test]
    fn test_rotating_logger_write_log_entry() {
        let path = tmp_log_path("log_entry");
        setup_log_dir(&path, 5);

        let logger = RotatingLogger::new(path.to_str().unwrap(), 1024, 3);
        let entry = LogEntry::new(Level::Info, "server started")
            .module("maharit_server")
            .field("port", "7687");
        logger.write(&entry);

        let contents = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["message"], "server started");
        assert_eq!(parsed["module"], "maharit_server");
        assert_eq!(parsed["port"], "7687");

        cleanup_log(&path, 3);
    }

    #[test]
    fn test_rotating_logger_rotates_on_size_exceeded() {
        let path = tmp_log_path("rotation");
        setup_log_dir(&path, 5);

        // Set a very small threshold so it rotates after the first write.
        let logger = RotatingLogger::new(path.to_str().unwrap(), 5, 3);

        // First write: file does not exist yet, no rotation.
        logger.write_entry("{\"seq\":1}");
        assert!(path.exists());

        // Second write: file already exceeds 5 bytes → rotation before appending.
        logger.write_entry("{\"seq\":2}");

        // After rotation the original file was moved to .1 and a new file was
        // created for the second entry.
        let rotated = PathBuf::from(format!("{}.1", path.display()));
        assert!(
            rotated.exists(),
            "rotated file {rotated:?} should exist after rotation"
        );

        let active_contents = fs::read_to_string(&path).unwrap();
        assert!(
            active_contents.contains("\"seq\":2"),
            "active log should contain the second entry: {active_contents}"
        );

        let rotated_contents = fs::read_to_string(&rotated).unwrap();
        assert!(
            rotated_contents.contains("\"seq\":1"),
            "rotated log should contain the first entry: {rotated_contents}"
        );

        cleanup_log(&path, 3);
    }

    #[test]
    fn test_rotating_logger_respects_max_generations() {
        let path = tmp_log_path("max_gen");
        setup_log_dir(&path, 5);

        // max_generations = 2 means we keep .1 and .2 only.
        let logger = RotatingLogger::new(path.to_str().unwrap(), 1, 2);

        // Write enough entries to trigger three rotations.
        for i in 0..4u32 {
            logger.write_entry(&format!("{{\"i\":{i}}}"));
        }

        // Generation .3 must not exist.
        let gen3 = PathBuf::from(format!("{}.3", path.display()));
        assert!(
            !gen3.exists(),
            "generation 3 should have been deleted: {gen3:?}"
        );

        cleanup_log(&path, 2);
    }

    #[test]
    fn test_rotating_logger_default_limits() {
        let logger = RotatingLogger::with_defaults("/tmp/maharit_default_test.log");
        assert_eq!(logger.max_size_bytes(), RotatingLogger::DEFAULT_MAX_SIZE);
        assert_eq!(logger.max_generations(), RotatingLogger::DEFAULT_MAX_GENERATIONS);
        assert_eq!(logger.log_file(), "/tmp/maharit_default_test.log");
    }
}
