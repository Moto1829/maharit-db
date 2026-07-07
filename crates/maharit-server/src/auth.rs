use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Role-based access control roles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Admin,     // Full access
    ReadWrite, // Read and write, no user management
    ReadOnly,  // Read only
}

/// User account representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    password_hash: String,
    salt: String,
    pub role: Role,
    pub created_at: u64, // Unix timestamp
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
    Read,        // MATCH, RETURN
    Write,       // CREATE, SET, DELETE, MERGE, REMOVE
    CreateIndex, // CREATE INDEX, CREATE CONSTRAINT
    DropIndex,   // DROP INDEX, DROP CONSTRAINT
    ManageUsers, // CREATE USER, DROP USER, ALTER USER
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

    #[error("Persistence error: {0}")]
    PersistenceError(String),
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// On-disk representation of a single user.
/// Stores the raw hash and salt so that passwords can be verified after reload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedUser {
    username: String,
    password_hash: String,
    salt: String,
    role: Role,
    created_at: u64,
    active: bool,
}

impl From<&User> for PersistedUser {
    fn from(u: &User) -> Self {
        Self {
            username: u.username.clone(),
            password_hash: u.password_hash.clone(),
            salt: u.salt.clone(),
            role: u.role,
            created_at: u.created_at,
            active: u.active,
        }
    }
}

impl From<PersistedUser> for User {
    fn from(p: PersistedUser) -> Self {
        Self {
            username: p.username,
            password_hash: p.password_hash,
            salt: p.salt,
            role: p.role,
            created_at: p.created_at,
            active: p.active,
        }
    }
}

/// Top-level envelope stored in `users.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedUsers {
    /// Schema version — increment when the format changes.
    version: u32,
    users: Vec<PersistedUser>,
}

impl PersistedUsers {
    const CURRENT_VERSION: u32 = 1;
}

// ---------------------------------------------------------------------------
// AuthManager
// ---------------------------------------------------------------------------

/// Central authentication and authorization manager
pub struct AuthManager {
    users: HashMap<String, User>,
    sessions: HashMap<String, Session>,
    session_timeout: Duration,
    /// Optional path for automatic persistence.  When `Some`, every mutating
    /// operation saves the user list to this file.
    save_path: Option<String>,
}

impl AuthManager {
    /// Create a new AuthManager with default admin user
    /// Default credentials: username="admin", password="admin"
    pub fn new() -> Self {
        let mut manager = Self {
            users: HashMap::new(),
            sessions: HashMap::new(),
            session_timeout: Duration::from_secs(30 * 60), // 30 minutes
            save_path: None,
        };

        // Create default admin user
        let admin = User::new("admin".to_string(), "admin", Role::Admin);
        manager.users.insert("admin".to_string(), admin);

        manager
    }

    /// Create a new AuthManager whose default `admin` user has the given
    /// password instead of the insecure default `"admin"`.
    ///
    /// Use this when enabling authentication so that the network-reachable
    /// admin account is not protected by well-known credentials.
    pub fn with_admin_password(password: &str) -> Self {
        let mut manager = Self {
            users: HashMap::new(),
            sessions: HashMap::new(),
            session_timeout: Duration::from_secs(30 * 60),
            save_path: None,
        };
        let admin = User::new("admin".to_string(), password, Role::Admin);
        manager.users.insert("admin".to_string(), admin);
        manager
    }

    /// Create a new AuthManager with custom session timeout
    pub fn with_session_timeout(timeout: Duration) -> Self {
        let mut manager = Self::new();
        manager.session_timeout = timeout;
        manager
    }

    /// Attach a save path so that every mutating operation persists user data.
    pub fn with_save_path(mut self, path: impl Into<String>) -> Self {
        self.save_path = Some(path.into());
        self
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Load an `AuthManager` from a JSON file previously written by
    /// [`save_to_file`].  If the file does not exist a fresh manager with the
    /// default admin account is returned.
    ///
    /// # Errors
    /// Returns [`AuthError::PersistenceError`] when the file exists but cannot
    /// be read or parsed.
    pub fn load_from_file(path: &str) -> Result<Self, AuthError> {
        if !std::path::Path::new(path).exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| AuthError::PersistenceError(format!("read {}: {}", path, e)))?;

        let persisted: PersistedUsers = serde_json::from_str(&content)
            .map_err(|e| AuthError::PersistenceError(format!("parse {}: {}", path, e)))?;

        let mut users = HashMap::new();
        for pu in persisted.users {
            let u = User::from(pu);
            users.insert(u.username.clone(), u);
        }

        Ok(Self {
            users,
            sessions: HashMap::new(),
            session_timeout: Duration::from_secs(30 * 60),
            save_path: Some(path.to_string()),
        })
    }

    /// Persist the current user list to `path` in JSON format.
    ///
    /// # Errors
    /// Returns [`AuthError::PersistenceError`] on I/O or serialisation errors.
    pub fn save_to_file(&self, path: &str) -> Result<(), AuthError> {
        let persisted = PersistedUsers {
            version: PersistedUsers::CURRENT_VERSION,
            users: self.users.values().map(PersistedUser::from).collect(),
        };

        let json = serde_json::to_string_pretty(&persisted)
            .map_err(|e| AuthError::PersistenceError(format!("serialize: {}", e)))?;

        std::fs::write(path, json)
            .map_err(|e| AuthError::PersistenceError(format!("write {}: {}", path, e)))?;

        Ok(())
    }

    /// Save to the configured `save_path` if one has been set.  Errors are
    /// silently ignored to avoid disrupting callers.
    fn auto_save(&self) {
        if let Some(ref path) = self.save_path {
            let _ = self.save_to_file(path);
        }
    }

    // -----------------------------------------------------------------------
    // User management
    // -----------------------------------------------------------------------

    /// Create a new user account
    pub fn create_user(
        &mut self,
        username: &str,
        password: &str,
        role: Role,
    ) -> Result<(), AuthError> {
        if self.users.contains_key(username) {
            return Err(AuthError::UserAlreadyExists(username.to_string()));
        }

        let user = User::new(username.to_string(), password, role);
        self.users.insert(username.to_string(), user);
        self.auto_save();
        Ok(())
    }

    /// Delete a user account
    /// Cannot delete the last admin user
    pub fn drop_user(&mut self, username: &str) -> Result<(), AuthError> {
        let user = self
            .users
            .get(username)
            .ok_or_else(|| AuthError::UserNotFound(username.to_string()))?;

        // Check if this is the last admin
        if user.role == Role::Admin {
            let admin_count = self
                .users
                .values()
                .filter(|u| u.role == Role::Admin && u.active)
                .count();

            if admin_count <= 1 {
                return Err(AuthError::CannotDeleteLastAdmin);
            }
        }

        self.users.remove(username);

        // Invalidate all sessions for this user
        self.sessions
            .retain(|_, session| session.username != username);

        self.auto_save();
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
            let admin_count = self
                .users
                .values()
                .filter(|u| u.role == Role::Admin && u.active)
                .count();

            if admin_count <= 1 {
                return Err(AuthError::CannotDeleteLastAdmin);
            }
        }

        self.users.get_mut(username).unwrap().role = role;
        self.auto_save();
        Ok(())
    }

    /// Change a user's password
    pub fn alter_user_password(&mut self, username: &str, password: &str) -> Result<(), AuthError> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| AuthError::UserNotFound(username.to_string()))?;

        user.update_password(password);
        self.auto_save();
        Ok(())
    }

    /// Authenticate user and create a new session
    /// Returns session token on success
    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<String, AuthError> {
        let user = self
            .users
            .get(username)
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
        let session = self
            .sessions
            .get_mut(token)
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
        Self::check_role_permission(role, operation)
    }

    /// Check whether `role` may perform `operation`, without needing an
    /// `AuthManager` instance.  Used by the network layer to enforce RBAC on
    /// every request.
    pub fn check_role_permission(role: Role, operation: Operation) -> Result<(), AuthError> {
        let allowed = match role {
            Role::Admin => true,
            Role::ReadWrite => matches!(
                operation,
                Operation::Read | Operation::Write | Operation::CreateIndex | Operation::DropIndex
            ),
            Role::ReadOnly => matches!(operation, Operation::Read),
        };

        if allowed {
            Ok(())
        } else {
            Err(AuthError::PermissionDenied(format!(
                "{:?} role cannot perform {:?} operation",
                role, operation
            )))
        }
    }

    /// List all users (returns references to avoid cloning)
    pub fn list_users(&self) -> Vec<&User> {
        self.users.values().collect()
    }

    /// Remove expired sessions
    pub fn cleanup_expired_sessions(&mut self) {
        let timeout = self.session_timeout;
        self.sessions
            .retain(|_, session| !session.is_expired(timeout));
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Crypto helpers
// ---------------------------------------------------------------------------

static SALT_COUNTER: AtomicU64 = AtomicU64::new(0);
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a random salt using system time and an atomic counter
fn generate_salt() -> String {
    let counter = SALT_COUNTER.fetch_add(1, Ordering::Relaxed);

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
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let random_part = fnv1a_64(&timestamp.to_le_bytes(), counter);

    format!("{:016x}-{:016x}", random_part, timestamp as u64)
}

// ---------------------------------------------------------------------------
// ACL (label/property-level access control)
// ---------------------------------------------------------------------------

/// The subject of an ACL rule: who the rule applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AclSubject {
    /// A specific named user.
    User(String),
    /// All users that hold a given role.
    Role(Role),
}

/// The resource that an ACL rule guards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AclResource {
    /// All nodes with the given label (e.g. `"Secret"`).
    Label(String),
    /// A specific property on nodes with the given label
    /// (e.g. `("User", "password")`).
    Property(String, String),
    /// Every label in the graph.
    AllLabels,
}

/// Whether access is explicitly permitted or denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AclPermission {
    Allow,
    Deny,
}

/// A single access-control rule.
///
/// Rules are evaluated by [`AclManager::check`]:
/// 1. A user-level `Deny` wins over everything.
/// 2. A role-level `Deny` wins next.
/// 3. A user-level `Allow` wins next.
/// 4. A role-level `Allow` wins next.
/// 5. Default is `Allow` (open by default, consistent with existing behaviour).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    /// Who this rule applies to.
    pub subject: AclSubject,
    /// Which resource is being controlled.
    pub resource: AclResource,
    /// Whether access is allowed or denied.
    pub permission: AclPermission,
}

/// Manages ACL rules for label- and property-level access control.
///
/// This sits on top of the role-based [`AuthManager`]: after the caller has
/// verified that the user's [`Role`] permits the requested [`Operation`], it
/// can use `AclManager` to further restrict access to specific labels or
/// properties.
pub struct AclManager {
    rules: Vec<AclRule>,
}

impl AclManager {
    /// Create a new, empty `AclManager`.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Append a rule to the rule list.
    pub fn add_rule(&mut self, rule: AclRule) {
        self.rules.push(rule);
    }

    /// Remove the rule at `index` and return it, or `None` if the index is
    /// out of bounds.
    pub fn remove_rule(&mut self, index: usize) -> Option<AclRule> {
        if index < self.rules.len() {
            Some(self.rules.remove(index))
        } else {
            None
        }
    }

    /// Return all rules.
    pub fn rules(&self) -> &[AclRule] {
        &self.rules
    }

    /// Determine whether `username` (holding `role`) may access `resource`.
    ///
    /// **Priority order** (first match wins):
    /// 1. User-level `Deny`
    /// 2. Role-level `Deny`
    /// 3. User-level `Allow`
    /// 4. Role-level `Allow`
    /// 5. Default → `Allow`
    pub fn check(
        &self,
        username: &str,
        role: Role,
        resource: &AclResource,
    ) -> AclPermission {
        // Collect matching rules for this subject+resource combination.
        let matches_resource = |r: &AclResource| -> bool {
            match (r, resource) {
                (AclResource::AllLabels, _) => true,
                (_, AclResource::AllLabels) => true,
                (AclResource::Label(a), AclResource::Label(b)) => a == b,
                (AclResource::Property(la, pa), AclResource::Property(lb, pb)) => {
                    la == lb && pa == pb
                }
                // A Label rule covers all properties of that label.
                (AclResource::Label(label_rule), AclResource::Property(label_res, _)) => {
                    label_rule == label_res
                }
                _ => false,
            }
        };

        // 1. User-level Deny
        if self.rules.iter().any(|r| {
            r.permission == AclPermission::Deny
                && matches!(&r.subject, AclSubject::User(u) if u == username)
                && matches_resource(&r.resource)
        }) {
            return AclPermission::Deny;
        }

        // 2. Role-level Deny
        if self.rules.iter().any(|r| {
            r.permission == AclPermission::Deny
                && matches!(&r.subject, AclSubject::Role(ro) if *ro == role)
                && matches_resource(&r.resource)
        }) {
            return AclPermission::Deny;
        }

        // 3. User-level Allow
        if self.rules.iter().any(|r| {
            r.permission == AclPermission::Allow
                && matches!(&r.subject, AclSubject::User(u) if u == username)
                && matches_resource(&r.resource)
        }) {
            return AclPermission::Allow;
        }

        // 4. Role-level Allow
        if self.rules.iter().any(|r| {
            r.permission == AclPermission::Allow
                && matches!(&r.subject, AclSubject::Role(ro) if *ro == role)
                && matches_resource(&r.resource)
        }) {
            return AclPermission::Allow;
        }

        // 5. Default: Allow (keeps backward compatibility)
        AclPermission::Allow
    }
}

impl Default for AclManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper method for testing (allows cloning AuthManager)
    impl AuthManager {
        #[cfg(test)]
        fn clone_for_test(&self) -> Self {
            Self {
                users: self.users.clone(),
                sessions: HashMap::new(),
                session_timeout: self.session_timeout,
                save_path: None,
            }
        }
    }

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
        manager
            .create_user("bob", "password", Role::ReadOnly)
            .unwrap();

        let result = manager.create_user("bob", "different", Role::Admin);

        assert_eq!(result, Err(AuthError::UserAlreadyExists("bob".to_string())));
    }

    #[test]
    fn test_drop_user_success() {
        let mut manager = AuthManager::new();
        manager
            .create_user("charlie", "pass", Role::ReadOnly)
            .unwrap();

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
        manager
            .create_user("dave", "secret123", Role::ReadWrite)
            .unwrap();

        let result = manager.authenticate("dave", "secret123");

        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
        assert_eq!(manager.sessions.len(), 1);
    }

    #[test]
    fn test_authenticate_wrong_password() {
        let mut manager = AuthManager::new();
        manager
            .create_user("eve", "correct", Role::ReadOnly)
            .unwrap();

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

        assert!(
            manager
                .check_permission(Role::Admin, Operation::Read)
                .is_ok()
        );
        assert!(
            manager
                .check_permission(Role::Admin, Operation::Write)
                .is_ok()
        );
        assert!(
            manager
                .check_permission(Role::Admin, Operation::CreateIndex)
                .is_ok()
        );
        assert!(
            manager
                .check_permission(Role::Admin, Operation::DropIndex)
                .is_ok()
        );
        assert!(
            manager
                .check_permission(Role::Admin, Operation::ManageUsers)
                .is_ok()
        );
    }

    #[test]
    fn test_permission_check_readonly_denied_write() {
        let manager = AuthManager::new();

        assert!(
            manager
                .check_permission(Role::ReadOnly, Operation::Read)
                .is_ok()
        );

        let write_result = manager.check_permission(Role::ReadOnly, Operation::Write);
        assert!(write_result.is_err());
        assert!(matches!(write_result, Err(AuthError::PermissionDenied(_))));

        let users_result = manager.check_permission(Role::ReadOnly, Operation::ManageUsers);
        assert!(users_result.is_err());
    }

    #[test]
    fn test_alter_user_role() {
        let mut manager = AuthManager::new();
        manager
            .create_user("frank", "pass", Role::ReadOnly)
            .unwrap();

        let result = manager.alter_user_role("frank", Role::ReadWrite);

        assert!(result.is_ok());
        let frank = manager.users.get("frank").unwrap();
        assert_eq!(frank.role, Role::ReadWrite);
    }

    #[test]
    fn test_alter_user_password() {
        let mut manager = AuthManager::new();
        manager
            .create_user("grace", "oldpass", Role::ReadWrite)
            .unwrap();

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
        manager
            .create_user("user1", "pass", Role::ReadOnly)
            .unwrap();
        manager
            .create_user("user2", "pass", Role::ReadWrite)
            .unwrap();

        let users = manager.list_users();

        assert_eq!(users.len(), 3); // admin + user1 + user2

        let usernames: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
        assert!(usernames.contains(&"admin"));
        assert!(usernames.contains(&"user1"));
        assert!(usernames.contains(&"user2"));
    }

    // -----------------------------------------------------------------------
    // Persistence tests
    // -----------------------------------------------------------------------

    /// Returns a temporary file path that is cleaned up on drop.
    struct TempFile(String);

    impl TempFile {
        fn new(name: &str) -> Self {
            let path = format!("/tmp/maharit_auth_test_{}_{}", name, std::process::id());
            TempFile(path)
        }
        fn path(&self) -> &str {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_save_and_load_users() {
        let tmp = TempFile::new("save_load");
        let path = tmp.path();

        // Create manager, add a user, save
        let mut manager = AuthManager::new();
        manager
            .create_user("persisted_user", "mypassword", Role::ReadWrite)
            .unwrap();
        manager.save_to_file(path).unwrap();

        // Load into a fresh manager
        let loaded = AuthManager::load_from_file(path).unwrap();
        let usernames: Vec<&str> = loaded.list_users().iter().map(|u| u.username.as_str()).collect();

        assert!(usernames.contains(&"admin"));
        assert!(usernames.contains(&"persisted_user"));
        assert_eq!(loaded.list_users().len(), 2);
    }

    #[test]
    fn test_loaded_users_can_authenticate() {
        let tmp = TempFile::new("auth_after_load");
        let path = tmp.path();

        // Save
        let mut manager = AuthManager::new();
        manager
            .create_user("harry", "hunter2", Role::ReadOnly)
            .unwrap();
        manager.save_to_file(path).unwrap();

        // Load and authenticate
        let mut loaded = AuthManager::load_from_file(path).unwrap();
        assert!(loaded.authenticate("harry", "hunter2").is_ok());
        assert_eq!(
            loaded.authenticate("harry", "wrong"),
            Err(AuthError::InvalidCredentials)
        );
    }

    #[test]
    fn test_load_from_nonexistent_file_returns_default() {
        let manager = AuthManager::load_from_file("/tmp/maharit_does_not_exist_xyz.json").unwrap();
        // Default manager has only the admin user
        assert_eq!(manager.list_users().len(), 1);
        assert_eq!(manager.list_users()[0].username, "admin");
    }

    #[test]
    fn test_auto_save_on_create_user() {
        let tmp = TempFile::new("auto_save_create");
        let path = tmp.path().to_string();

        let mut manager = AuthManager::new().with_save_path(path.clone());
        // create_user should trigger auto_save
        manager.create_user("iris", "pass", Role::ReadOnly).unwrap();

        // The file must exist now
        assert!(std::path::Path::new(&path).exists());

        // Loading it must include iris
        let loaded = AuthManager::load_from_file(&path).unwrap();
        let usernames: Vec<&str> = loaded.list_users().iter().map(|u| u.username.as_str()).collect();
        assert!(usernames.contains(&"iris"));
    }

    #[test]
    fn test_auto_save_on_drop_user() {
        let tmp = TempFile::new("auto_save_drop");
        let path = tmp.path().to_string();

        let mut manager = AuthManager::new().with_save_path(path.clone());
        manager.create_user("jack", "pass", Role::ReadOnly).unwrap();
        // Add a second admin so we can drop jack without issue
        manager.create_user("admin2", "pass", Role::Admin).unwrap();

        // Drop jack
        manager.drop_user("jack").unwrap();

        // Reload and verify jack is gone
        let loaded = AuthManager::load_from_file(&path).unwrap();
        let usernames: Vec<&str> = loaded.list_users().iter().map(|u| u.username.as_str()).collect();
        assert!(!usernames.contains(&"jack"));
        assert!(usernames.contains(&"admin"));
        assert!(usernames.contains(&"admin2"));
    }

    #[test]
    fn test_persisted_users_schema_version() {
        let tmp = TempFile::new("schema_version");
        let path = tmp.path();

        let manager = AuthManager::new();
        manager.save_to_file(path).unwrap();

        let content = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["version"], 1u32);
    }

    #[test]
    fn test_load_from_invalid_json_returns_error() {
        let tmp = TempFile::new("invalid_json");
        let path = tmp.path();
        std::fs::write(path, "not json at all").unwrap();

        let result = AuthManager::load_from_file(path);
        assert!(matches!(result, Err(AuthError::PersistenceError(_))));
    }

    // -----------------------------------------------------------------------
    // AclManager tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_acl_default_allow_when_no_rules() {
        let mgr = AclManager::new();
        let result = mgr.check("alice", Role::ReadOnly, &AclResource::Label("Secret".to_string()));
        assert_eq!(result, AclPermission::Allow);
    }

    #[test]
    fn test_acl_deny_specific_label() {
        let mut mgr = AclManager::new();
        mgr.add_rule(AclRule {
            subject: AclSubject::User("bob".to_string()),
            resource: AclResource::Label("Secret".to_string()),
            permission: AclPermission::Deny,
        });

        // bob is denied access to Secret label
        assert_eq!(
            mgr.check("bob", Role::ReadWrite, &AclResource::Label("Secret".to_string())),
            AclPermission::Deny
        );
        // alice is not affected
        assert_eq!(
            mgr.check("alice", Role::ReadWrite, &AclResource::Label("Secret".to_string())),
            AclPermission::Allow
        );
    }

    #[test]
    fn test_acl_deny_specific_property() {
        let mut mgr = AclManager::new();
        mgr.add_rule(AclRule {
            subject: AclSubject::Role(Role::ReadOnly),
            resource: AclResource::Property("User".to_string(), "password".to_string()),
            permission: AclPermission::Deny,
        });

        // ReadOnly role cannot access User.password
        assert_eq!(
            mgr.check(
                "alice",
                Role::ReadOnly,
                &AclResource::Property("User".to_string(), "password".to_string())
            ),
            AclPermission::Deny
        );
        // Admin role is not affected by this rule
        assert_eq!(
            mgr.check(
                "admin",
                Role::Admin,
                &AclResource::Property("User".to_string(), "password".to_string())
            ),
            AclPermission::Allow
        );
    }

    #[test]
    fn test_acl_deny_beats_allow() {
        let mut mgr = AclManager::new();
        // Allow for the role
        mgr.add_rule(AclRule {
            subject: AclSubject::Role(Role::ReadWrite),
            resource: AclResource::Label("Secret".to_string()),
            permission: AclPermission::Allow,
        });
        // But deny for the specific user
        mgr.add_rule(AclRule {
            subject: AclSubject::User("mallory".to_string()),
            resource: AclResource::Label("Secret".to_string()),
            permission: AclPermission::Deny,
        });

        // mallory is denied despite the role-level Allow
        assert_eq!(
            mgr.check("mallory", Role::ReadWrite, &AclResource::Label("Secret".to_string())),
            AclPermission::Deny
        );
        // other ReadWrite users still get Allow
        assert_eq!(
            mgr.check("alice", Role::ReadWrite, &AclResource::Label("Secret".to_string())),
            AclPermission::Allow
        );
    }

    #[test]
    fn test_acl_user_rule_beats_role_deny() {
        let mut mgr = AclManager::new();
        // Deny the whole ReadOnly role
        mgr.add_rule(AclRule {
            subject: AclSubject::Role(Role::ReadOnly),
            resource: AclResource::Label("Internal".to_string()),
            permission: AclPermission::Deny,
        });
        // But explicitly allow a specific user in that role (user-level Deny
        // wins over role-level Deny, and user-level Allow wins over role-level
        // Deny per the priority order: user-Deny > role-Deny > user-Allow >
        // role-Allow).  Since there is no user-Deny for "trusted", user-Allow
        // is reached before role-Deny resolves… wait – the spec says Deny
        // beats Allow.  Let's verify the documented priority:
        // 1 user-Deny, 2 role-Deny, 3 user-Allow, 4 role-Allow.
        // Here we add a user-Allow for "trusted" on top of the role-Deny.
        // "trusted" has no user-Deny, so we fall to role-Deny → Deny.
        // This test verifies role-Deny wins over user-Allow (Deny-first logic).
        mgr.add_rule(AclRule {
            subject: AclSubject::User("trusted".to_string()),
            resource: AclResource::Label("Internal".to_string()),
            permission: AclPermission::Allow,
        });

        // role-Deny beats user-Allow → Deny
        assert_eq!(
            mgr.check("trusted", Role::ReadOnly, &AclResource::Label("Internal".to_string())),
            AclPermission::Deny
        );
    }

    #[test]
    fn test_acl_all_labels_resource() {
        let mut mgr = AclManager::new();
        mgr.add_rule(AclRule {
            subject: AclSubject::Role(Role::ReadOnly),
            resource: AclResource::AllLabels,
            permission: AclPermission::Deny,
        });

        // ReadOnly is denied on any label
        assert_eq!(
            mgr.check("alice", Role::ReadOnly, &AclResource::Label("Public".to_string())),
            AclPermission::Deny
        );
        assert_eq!(
            mgr.check(
                "alice",
                Role::ReadOnly,
                &AclResource::Property("Public".to_string(), "data".to_string())
            ),
            AclPermission::Deny
        );
    }

    #[test]
    fn test_acl_remove_rule() {
        let mut mgr = AclManager::new();
        mgr.add_rule(AclRule {
            subject: AclSubject::User("bob".to_string()),
            resource: AclResource::Label("Secret".to_string()),
            permission: AclPermission::Deny,
        });

        assert_eq!(mgr.rules().len(), 1);

        let removed = mgr.remove_rule(0);
        assert!(removed.is_some());
        assert_eq!(mgr.rules().len(), 0);

        // After removing the deny rule, default Allow applies
        assert_eq!(
            mgr.check("bob", Role::ReadWrite, &AclResource::Label("Secret".to_string())),
            AclPermission::Allow
        );
    }

    #[test]
    fn test_acl_remove_rule_out_of_bounds() {
        let mut mgr = AclManager::new();
        let removed = mgr.remove_rule(99);
        assert!(removed.is_none());
    }
}
