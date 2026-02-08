use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};
use thiserror::Error;

/// Role-based access control roles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Admin,      // Full access
    ReadWrite,  // Read and write, no user management
    ReadOnly,   // Read only
}

/// User account representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    password_hash: String,
    salt: String,
    pub role: Role,
    pub created_at: u64,  // Unix timestamp
    pub active: bool,
}

impl User {
    /// Create a new user with hashed password
    fn new(username: String, password: &str, role: Role) -> Self {
        let salt = generate_salt();
        let password_hash = hash_password(&salt, password);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            username,
            password_hash,
            salt,
            role,
            created_at,
            active: true,
        }
    }

    /// Verify password against stored hash
    fn verify_password(&self, password: &str) -> bool {
        let computed_hash = hash_password(&self.salt, password);
        computed_hash == self.password_hash
    }

    /// Update password with new hash and salt
    fn update_password(&mut self, password: &str) {
        self.salt = generate_salt();
        self.password_hash = hash_password(&self.salt, password);
    }
}

/// Active session representation
pub struct Session {
    pub token: String,
    pub username: String,
    pub role: Role,
    pub created_at: Instant,
    pub last_active: Instant,
}

impl Session {
    /// Create a new session with generated token
    fn new(username: String, role: Role) -> Self {
        let token = generate_token();
        let now = Instant::now();

        Self {
            token,
            username,
            role,
            created_at: now,
            last_active: now,
        }
    }

    /// Check if session has expired based on timeout duration
    fn is_expired(&self, timeout: Duration) -> bool {
        self.last_active.elapsed() > timeout
    }

    /// Update last active timestamp
    fn refresh(&mut self) {
        self.last_active = Instant::now();
    }
}

/// Database operations requiring authorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Read,           // MATCH, RETURN
    Write,          // CREATE, SET, DELETE, MERGE, REMOVE
    CreateIndex,    // CREATE INDEX, CREATE CONSTRAINT
    DropIndex,      // DROP INDEX, DROP CONSTRAINT
    ManageUsers,    // CREATE USER, DROP USER, ALTER USER
}

/// Authentication and authorization errors
#[derive(Debug, Clone, Error, PartialEq)]
pub enum AuthError {
    #[error("User not found: {0}")]
    UserNotFound(String),

    #[error("User already exists: {0}")]
    UserAlreadyExists(String),

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Session expired")]
    SessionExpired,

    #[error("Invalid session")]
    InvalidSession,

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Cannot delete the last admin user")]
    CannotDeleteLastAdmin,
}

/// Central authentication and authorization manager
pub struct AuthManager {
    users: HashMap<String, User>,
    sessions: HashMap<String, Session>,
    session_timeout: Duration,
}

impl AuthManager {
    /// Create a new AuthManager with default admin user
    /// Default credentials: username="admin", password="admin"
    pub fn new() -> Self {
        let mut manager = Self {
            users: HashMap::new(),
            sessions: HashMap::new(),
            session_timeout: Duration::from_secs(30 * 60), // 30 minutes
        };

        // Create default admin user
        let admin = User::new("admin".to_string(), "admin", Role::Admin);
        manager.users.insert("admin".to_string(), admin);

        manager
    }

    /// Create a new AuthManager with custom session timeout
    pub fn with_session_timeout(timeout: Duration) -> Self {
        let mut manager = Self::new();
        manager.session_timeout = timeout;
        manager
    }

    /// Create a new user account
    pub fn create_user(&mut self, username: &str, password: &str, role: Role) -> Result<(), AuthError> {
        if self.users.contains_key(username) {
            return Err(AuthError::UserAlreadyExists(username.to_string()));
        }

        let user = User::new(username.to_string(), password, role);
        self.users.insert(username.to_string(), user);
        Ok(())
    }

    /// Delete a user account
    /// Cannot delete the last admin user
    pub fn drop_user(&mut self, username: &str) -> Result<(), AuthError> {
        let user = self.users.get(username)
            .ok_or_else(|| AuthError::UserNotFound(username.to_string()))?;

        // Check if this is the last admin
        if user.role == Role::Admin {
            let admin_count = self.users.values()
                .filter(|u| u.role == Role::Admin && u.active)
                .count();

            if admin_count <= 1 {
                return Err(AuthError::CannotDeleteLastAdmin);
            }
        }

        self.users.remove(username);

        // Invalidate all sessions for this user
        self.sessions.retain(|_, session| session.username != username);

        Ok(())
    }

    /// Change a user's role
    pub fn alter_user_role(&mut self, username: &str, role: Role) -> Result<(), AuthError> {
        if !self.users.contains_key(username) {
            return Err(AuthError::UserNotFound(username.to_string()));
        }

        // Check if changing from admin would leave no admins
        let current_role = self.users[username].role;
        if current_role == Role::Admin && role != Role::Admin {
            let admin_count = self.users.values()
                .filter(|u| u.role == Role::Admin && u.active)
                .count();

            if admin_count <= 1 {
                return Err(AuthError::CannotDeleteLastAdmin);
            }
        }

        self.users.get_mut(username).unwrap().role = role;
        Ok(())
    }

    /// Change a user's password
    pub fn alter_user_password(&mut self, username: &str, password: &str) -> Result<(), AuthError> {
        let user = self.users.get_mut(username)
            .ok_or_else(|| AuthError::UserNotFound(username.to_string()))?;

        user.update_password(password);
        Ok(())
    }

    /// Authenticate user and create a new session
    /// Returns session token on success
    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<String, AuthError> {
        let user = self.users.get(username)
            .ok_or(AuthError::InvalidCredentials)?;

        if !user.active {
            return Err(AuthError::InvalidCredentials);
        }

        if !user.verify_password(password) {
            return Err(AuthError::InvalidCredentials);
        }

        let session = Session::new(username.to_string(), user.role);
        let token = session.token.clone();
        self.sessions.insert(token.clone(), session);

        Ok(token)
    }

    /// Validate a session token and refresh its timeout
    pub fn validate_session(&mut self, token: &str) -> Result<&Session, AuthError> {
        let session = self.sessions.get_mut(token)
            .ok_or(AuthError::InvalidSession)?;

        if session.is_expired(self.session_timeout) {
            self.sessions.remove(token);
            return Err(AuthError::SessionExpired);
        }

        session.refresh();
        Ok(self.sessions.get(token).unwrap())
    }

    /// Invalidate a session (logout)
    pub fn invalidate_session(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    /// Check if a role has permission for an operation
    pub fn check_permission(&self, role: Role, operation: Operation) -> Result<(), AuthError> {
        let allowed = match role {
            Role::Admin => true,
            Role::ReadWrite => matches!(operation,
                Operation::Read | Operation::Write | Operation::CreateIndex | Operation::DropIndex
            ),
            Role::ReadOnly => matches!(operation, Operation::Read),
        };

        if allowed {
            Ok(())
        } else {
            Err(AuthError::PermissionDenied(
                format!("{:?} role cannot perform {:?} operation", role, operation)
            ))
        }
    }

    /// List all users (returns references to avoid cloning)
    pub fn list_users(&self) -> Vec<&User> {
        self.users.values().collect()
    }

    /// Remove expired sessions
    pub fn cleanup_expired_sessions(&mut self) {
        let timeout = self.session_timeout;
        self.sessions.retain(|_, session| !session.is_expired(timeout));
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a random salt using system time and a counter
fn generate_salt() -> String {
    static mut COUNTER: u64 = 0;
    let counter = unsafe {
        COUNTER += 1;
        COUNTER
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    format!("{:x}{:x}", timestamp, counter)
}

/// Hash password with salt using FNV-1a variant
fn hash_password(salt: &str, password: &str) -> String {
    let input = format!("{}{}", salt, password);
    let bytes = input.as_bytes();

    // FNV-1a 64-bit hash with multiple rounds for longer output
    let seeds = [
        0xcbf29ce484222325u64,
        0x84222325cbf29ce4u64,
        0x9ce484222325cbf2u64,
        0x2325cbf29ce48422u64,
    ];

    let mut result = String::new();
    for seed in &seeds {
        let hash = fnv1a_64(bytes, *seed);
        result.push_str(&format!("{:016x}", hash));
    }

    result
}

/// FNV-1a 64-bit hash function
fn fnv1a_64(data: &[u8], seed: u64) -> u64 {
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = seed;

    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    hash
}

/// Generate a session token
fn generate_token() -> String {
    static mut COUNTER: u64 = 0;
    let counter = unsafe {
        COUNTER += 1;
        COUNTER
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let random_part = fnv1a_64(&timestamp.to_le_bytes(), counter);

    format!("{:016x}-{:016x}", random_part, timestamp as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_admin_user_creation() {
        let manager = AuthManager::new();
        let users = manager.list_users();

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "admin");
        assert_eq!(users[0].role, Role::Admin);
        assert!(users[0].active);
    }

    #[test]
    fn test_create_user_success() {
        let mut manager = AuthManager::new();
        let result = manager.create_user("alice", "password123", Role::ReadWrite);

        assert!(result.is_ok());
        assert_eq!(manager.list_users().len(), 2);

        let alice = manager.users.get("alice").unwrap();
        assert_eq!(alice.username, "alice");
        assert_eq!(alice.role, Role::ReadWrite);
    }

    #[test]
    fn test_create_duplicate_user_error() {
        let mut manager = AuthManager::new();
        manager.create_user("bob", "password", Role::ReadOnly).unwrap();

        let result = manager.create_user("bob", "different", Role::Admin);

        assert_eq!(result, Err(AuthError::UserAlreadyExists("bob".to_string())));
    }

    #[test]
    fn test_drop_user_success() {
        let mut manager = AuthManager::new();
        manager.create_user("charlie", "pass", Role::ReadOnly).unwrap();

        assert_eq!(manager.list_users().len(), 2);

        let result = manager.drop_user("charlie");
        assert!(result.is_ok());
        assert_eq!(manager.list_users().len(), 1);
    }

    #[test]
    fn test_drop_last_admin_error() {
        let manager = AuthManager::new();

        // Only one admin exists (default admin)
        let result = manager.clone_for_test().drop_user("admin");

        assert_eq!(result, Err(AuthError::CannotDeleteLastAdmin));
    }

    #[test]
    fn test_authenticate_success() {
        let mut manager = AuthManager::new();
        manager.create_user("dave", "secret123", Role::ReadWrite).unwrap();

        let result = manager.authenticate("dave", "secret123");

        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
        assert_eq!(manager.sessions.len(), 1);
    }

    #[test]
    fn test_authenticate_wrong_password() {
        let mut manager = AuthManager::new();
        manager.create_user("eve", "correct", Role::ReadOnly).unwrap();

        let result = manager.authenticate("eve", "wrong");

        assert_eq!(result, Err(AuthError::InvalidCredentials));
    }

    #[test]
    fn test_authenticate_nonexistent_user() {
        let mut manager = AuthManager::new();

        let result = manager.authenticate("nobody", "password");

        assert_eq!(result, Err(AuthError::InvalidCredentials));
    }

    #[test]
    fn test_session_validation() {
        let mut manager = AuthManager::new();
        let token = manager.authenticate("admin", "admin").unwrap();

        let result = manager.validate_session(&token);

        assert!(result.is_ok());
        let session = result.unwrap();
        assert_eq!(session.username, "admin");
        assert_eq!(session.role, Role::Admin);
    }

    #[test]
    fn test_permission_check_admin_all_ops() {
        let manager = AuthManager::new();

        assert!(manager.check_permission(Role::Admin, Operation::Read).is_ok());
        assert!(manager.check_permission(Role::Admin, Operation::Write).is_ok());
        assert!(manager.check_permission(Role::Admin, Operation::CreateIndex).is_ok());
        assert!(manager.check_permission(Role::Admin, Operation::DropIndex).is_ok());
        assert!(manager.check_permission(Role::Admin, Operation::ManageUsers).is_ok());
    }

    #[test]
    fn test_permission_check_readonly_denied_write() {
        let manager = AuthManager::new();

        assert!(manager.check_permission(Role::ReadOnly, Operation::Read).is_ok());

        let write_result = manager.check_permission(Role::ReadOnly, Operation::Write);
        assert!(write_result.is_err());
        assert!(matches!(write_result, Err(AuthError::PermissionDenied(_))));

        let users_result = manager.check_permission(Role::ReadOnly, Operation::ManageUsers);
        assert!(users_result.is_err());
    }

    #[test]
    fn test_alter_user_role() {
        let mut manager = AuthManager::new();
        manager.create_user("frank", "pass", Role::ReadOnly).unwrap();

        let result = manager.alter_user_role("frank", Role::ReadWrite);

        assert!(result.is_ok());
        let frank = manager.users.get("frank").unwrap();
        assert_eq!(frank.role, Role::ReadWrite);
    }

    #[test]
    fn test_alter_user_password() {
        let mut manager = AuthManager::new();
        manager.create_user("grace", "oldpass", Role::ReadWrite).unwrap();

        // Old password works
        assert!(manager.authenticate("grace", "oldpass").is_ok());

        // Change password
        let result = manager.alter_user_password("grace", "newpass");
        assert!(result.is_ok());

        // Old password no longer works
        assert_eq!(
            manager.authenticate("grace", "oldpass"),
            Err(AuthError::InvalidCredentials)
        );

        // New password works
        assert!(manager.authenticate("grace", "newpass").is_ok());
    }

    #[test]
    fn test_list_users() {
        let mut manager = AuthManager::new();
        manager.create_user("user1", "pass", Role::ReadOnly).unwrap();
        manager.create_user("user2", "pass", Role::ReadWrite).unwrap();

        let users = manager.list_users();

        assert_eq!(users.len(), 3); // admin + user1 + user2

        let usernames: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
        assert!(usernames.contains(&"admin"));
        assert!(usernames.contains(&"user1"));
        assert!(usernames.contains(&"user2"));
    }

    // Helper method for testing (allows cloning AuthManager)
    impl AuthManager {
        #[cfg(test)]
        fn clone_for_test(&self) -> Self {
            Self {
                users: self.users.clone(),
                sessions: HashMap::new(),
                session_timeout: self.session_timeout,
            }
        }
    }
}
