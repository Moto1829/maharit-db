//! Audit logging for authentication events and query execution.
//!
//! Each log line is a self-contained JSON object terminated by a newline:
//!
//! ```text
//! {"timestamp":1700000000,"event":"login_success","username":"alice"}
//! {"timestamp":1700000001,"event":"query","user":"alice","query":"MATCH (n) RETURN n","success":true}
//! ```
//!
//! Log files are rotated when they exceed `max_size_bytes`.  Old generations
//! are renamed `<base>.1`, `<base>.2`, … up to `max_generations`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// AuthEvent
// ---------------------------------------------------------------------------

/// An authentication or user-management event to be recorded in the audit log.
#[derive(Debug, Clone)]
pub enum AuthEvent {
    /// A user successfully authenticated.
    LoginSuccess { username: String },
    /// An authentication attempt failed.
    LoginFailure { username: String },
    /// A session was started (token issued).
    SessionStarted { username: String },
    /// A session was invalidated (logout or expiry).
    SessionEnded { username: String },
    /// A new user account was created.
    UserCreated { username: String, by: String },
    /// A user account was deleted.
    UserDeleted { username: String, by: String },
    /// A user account was modified (role or password change).
    UserModified { username: String, by: String },
}

impl AuthEvent {
    /// Return the event type string used as the `"event"` field in JSON output.
    fn event_type(&self) -> &'static str {
        match self {
            AuthEvent::LoginSuccess { .. } => "login_success",
            AuthEvent::LoginFailure { .. } => "login_failure",
            AuthEvent::SessionStarted { .. } => "session_started",
            AuthEvent::SessionEnded { .. } => "session_ended",
            AuthEvent::UserCreated { .. } => "user_created",
            AuthEvent::UserDeleted { .. } => "user_deleted",
            AuthEvent::UserModified { .. } => "user_modified",
        }
    }

    /// Serialize the event-specific fields (excluding `timestamp` and `event`)
    /// as a JSON fragment, e.g. `"username":"alice","by":"admin"`.
    fn fields_json(&self) -> String {
        match self {
            AuthEvent::LoginSuccess { username }
            | AuthEvent::LoginFailure { username }
            | AuthEvent::SessionStarted { username }
            | AuthEvent::SessionEnded { username } => {
                format!(",\"username\":\"{}\"", escape_json(username))
            }
            AuthEvent::UserCreated { username, by }
            | AuthEvent::UserDeleted { username, by }
            | AuthEvent::UserModified { username, by } => {
                format!(
                    ",\"username\":\"{}\",\"by\":\"{}\"",
                    escape_json(username),
                    escape_json(by)
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AuditLogger
// ---------------------------------------------------------------------------

/// Appends structured audit records to a log file and rotates it when the
/// file exceeds `max_size_bytes`.
///
/// # Thread safety
/// `AuditLogger` uses interior mutability via a `std::sync::Mutex` around the
/// underlying file handle so it can be shared across threads through `Arc`.
/// All public methods take `&self`.
#[derive(Debug)]
pub struct AuditLogger {
    log_file: String,
    max_size_bytes: u64,
    max_generations: u32,
    inner: std::sync::Mutex<()>, // serialises concurrent writes
}

impl AuditLogger {
    /// Create a new `AuditLogger`.
    ///
    /// * `log_file`        – path to the log file (created on first write).
    /// * `max_size_bytes`  – rotate when the file exceeds this size (bytes).
    /// * `max_generations` – maximum number of rotated archives to keep.
    pub fn new(log_file: impl Into<String>, max_size_bytes: u64, max_generations: u32) -> Self {
        Self {
            log_file: log_file.into(),
            max_size_bytes,
            max_generations,
            inner: std::sync::Mutex::new(()),
        }
    }

    /// Create an `AuditLogger` with sensible defaults:
    /// * `max_size_bytes  = 10 * 1024 * 1024` (10 MiB)
    /// * `max_generations = 3`
    pub fn with_defaults(log_file: impl Into<String>) -> Self {
        Self::new(log_file, 10 * 1024 * 1024, 3)
    }

    /// Record an authentication or user-management event.
    pub fn log_auth_event(&self, event: AuthEvent) {
        let timestamp = unix_timestamp();
        let line = format!(
            "{{\"timestamp\":{},\"event\":\"{}\"{}}}\n",
            timestamp,
            event.event_type(),
            event.fields_json()
        );
        self.append_line(&line);
    }

    /// Record a query execution attempt.
    ///
    /// * `user`    – the authenticated username.
    /// * `query`   – the query string that was executed.
    /// * `success` – whether the execution succeeded.
    pub fn log_query(&self, user: &str, query: &str, success: bool) {
        let timestamp = unix_timestamp();
        let line = format!(
            "{{\"timestamp\":{},\"event\":\"query\",\"user\":\"{}\",\"query\":\"{}\",\"success\":{}}}\n",
            timestamp,
            escape_json(user),
            escape_json(query),
            success,
        );
        self.append_line(&line);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Append a line to the log file.  Rotates first if needed.
    fn append_line(&self, line: &str) {
        // Serialise access so concurrent callers don't interleave lines.
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
    ///
    /// Rotation scheme:
    /// ```text
    /// audit.log.2 → deleted (if max_generations == 2)
    /// audit.log.1 → audit.log.2
    /// audit.log   → audit.log.1
    /// (new empty audit.log created on next write)
    /// ```
    fn rotate_if_needed(&self) {
        let current_size = match std::fs::metadata(&self.log_file) {
            Ok(m) => m.len(),
            Err(_) => return, // file doesn't exist yet
        };

        if current_size < self.max_size_bytes {
            return;
        }

        // Remove the oldest generation that would overflow the limit
        let oldest = format!("{}.{}", self.log_file, self.max_generations);
        let _ = std::fs::remove_file(&oldest);

        // Shift each generation up by one
        for generation in (1..self.max_generations).rev() {
            let from = format!("{}.{}", self.log_file, generation);
            let to = format!("{}.{}", self.log_file, generation + 1);
            let _ = std::fs::rename(&from, &to);
        }

        // Move current log → .1
        let _ = std::fs::rename(&self.log_file, format!("{}.1", self.log_file));

        // Create a fresh, empty log file so the next append succeeds.
        let _ = File::create(&self.log_file);
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Return the current Unix timestamp in whole seconds.
fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Escape a string for embedding inside a JSON string literal.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    struct TempLog(String);

    impl TempLog {
        fn new(label: &str) -> Self {
            TempLog(format!(
                "/tmp/maharit_audit_test_{}_{}",
                label,
                std::process::id()
            ))
        }
        fn path(&self) -> &str {
            &self.0
        }
    }

    impl Drop for TempLog {
        fn drop(&mut self) {
            // Remove the main log and any rotated generations
            let _ = fs::remove_file(&self.0);
            for i in 1..=10 {
                let _ = fs::remove_file(format!("{}.{}", self.0, i));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Auth event tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_log_auth_event_login_success() {
        let tmp = TempLog::new("login_success");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::LoginSuccess {
            username: "alice".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("\"event\":\"login_success\""));
        assert!(content.contains("\"username\":\"alice\""));
        assert!(content.contains("\"timestamp\":"));

        // Must be valid JSON
        let line = content.lines().next().unwrap();
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["event"], "login_success");
        assert_eq!(v["username"], "alice");
    }

    #[test]
    fn test_log_auth_event_login_failure() {
        let tmp = TempLog::new("login_failure");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::LoginFailure {
            username: "bob".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["event"], "login_failure");
        assert_eq!(v["username"], "bob");
    }

    #[test]
    fn test_log_auth_event_user_created() {
        let tmp = TempLog::new("user_created");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::UserCreated {
            username: "charlie".to_string(),
            by: "admin".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["event"], "user_created");
        assert_eq!(v["username"], "charlie");
        assert_eq!(v["by"], "admin");
    }

    #[test]
    fn test_log_auth_event_user_deleted() {
        let tmp = TempLog::new("user_deleted");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::UserDeleted {
            username: "dave".to_string(),
            by: "admin".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["event"], "user_deleted");
        assert_eq!(v["username"], "dave");
        assert_eq!(v["by"], "admin");
    }

    #[test]
    fn test_log_auth_event_session_events() {
        let tmp = TempLog::new("session_events");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::SessionStarted {
            username: "eve".to_string(),
        });
        logger.log_auth_event(AuthEvent::SessionEnded {
            username: "eve".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let v0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let v1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v0["event"], "session_started");
        assert_eq!(v1["event"], "session_ended");
    }

    #[test]
    fn test_log_auth_event_user_modified() {
        let tmp = TempLog::new("user_modified");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_auth_event(AuthEvent::UserModified {
            username: "frank".to_string(),
            by: "admin".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["event"], "user_modified");
    }

    // -----------------------------------------------------------------------
    // Query log tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_log_query_success() {
        let tmp = TempLog::new("query_success");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_query("alice", "MATCH (n) RETURN n", true);

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["event"], "query");
        assert_eq!(v["user"], "alice");
        assert_eq!(v["query"], "MATCH (n) RETURN n");
        assert_eq!(v["success"], true);
    }

    #[test]
    fn test_log_query_failure() {
        let tmp = TempLog::new("query_failure");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_query("bob", "INVALID QUERY", false);

        let content = fs::read_to_string(tmp.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["success"], false);
    }

    #[test]
    fn test_log_query_escapes_special_characters() {
        let tmp = TempLog::new("query_escape");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_query("alice", "MATCH (n {name:\"Alice\"}) RETURN n", true);

        let content = fs::read_to_string(tmp.path()).unwrap();
        // Must parse as valid JSON
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert!(v["query"].as_str().unwrap().contains("Alice"));
    }

    #[test]
    fn test_multiple_log_entries_one_per_line() {
        let tmp = TempLog::new("multi_line");
        let logger = AuditLogger::with_defaults(tmp.path());

        logger.log_query("alice", "MATCH (n) RETURN n", true);
        logger.log_query("bob", "CREATE (n:Person)", true);
        logger.log_auth_event(AuthEvent::LoginFailure {
            username: "charlie".to_string(),
        });

        let content = fs::read_to_string(tmp.path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);

        // Every line must be valid JSON
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed["timestamp"].is_number());
            assert!(parsed["event"].is_string());
        }
    }

    // -----------------------------------------------------------------------
    // Log rotation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rotation_triggered_by_size() {
        let tmp = TempLog::new("rotation_size");
        let path = tmp.path();

        // Use a tiny max size so rotation triggers immediately
        let logger = AuditLogger::new(path, 50, 3);

        // Write enough data to trigger rotation
        for _ in 0..20 {
            logger.log_query("alice", "MATCH (n) RETURN n", true);
        }

        // After rotation the `.1` archive must exist
        assert!(
            std::path::Path::new(&format!("{}.1", path)).exists(),
            "rotated file {}.1 should exist",
            path
        );
    }

    #[test]
    fn test_rotation_shifts_generations() {
        let tmp = TempLog::new("rotation_shift");
        let path = tmp.path();

        // 1-byte limit so every write triggers a rotation
        let logger = AuditLogger::new(path, 1, 3);

        // Enough writes to fill several generations
        for i in 0..10 {
            logger.log_query("alice", &format!("QUERY {}", i), true);
        }

        // Generation 1 and 2 must both exist (generation 3 may have been
        // removed as the oldest before being promoted)
        assert!(
            std::path::Path::new(&format!("{}.1", path)).exists(),
            "{}.1 must exist",
            path
        );
    }

    #[test]
    fn test_rotation_respects_max_generations() {
        let tmp = TempLog::new("rotation_max");
        let path = tmp.path();

        // Only keep 2 generations
        let logger = AuditLogger::new(path, 1, 2);

        for i in 0..20 {
            logger.log_query("alice", &format!("QUERY {}", i), true);
        }

        // Generation 3 must NOT exist
        assert!(
            !std::path::Path::new(&format!("{}.3", path)).exists(),
            "{}.3 should not exist when max_generations=2",
            path
        );
    }

    #[test]
    fn test_no_rotation_below_threshold() {
        let tmp = TempLog::new("no_rotation");
        let path = tmp.path();

        // Very large threshold — no rotation should occur
        let logger = AuditLogger::new(path, 100 * 1024 * 1024, 3);

        for _ in 0..5 {
            logger.log_query("alice", "MATCH (n) RETURN n", true);
        }

        // No rotated file should exist
        assert!(
            !std::path::Path::new(&format!("{}.1", path)).exists(),
            "{}.1 should not exist",
            path
        );
    }

    // -----------------------------------------------------------------------
    // escape_json helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_escape_json_plain() {
        assert_eq!(escape_json("hello"), "hello");
    }

    #[test]
    fn test_escape_json_quotes() {
        assert_eq!(escape_json("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_escape_json_newline() {
        assert_eq!(escape_json("a\nb"), "a\\nb");
    }

    #[test]
    fn test_escape_json_backslash() {
        assert_eq!(escape_json("a\\b"), "a\\\\b");
    }
}
