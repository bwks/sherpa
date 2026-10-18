//! User table schema definition
//!
//! The user table stores user accounts with password authentication and optional SSH keys.
//! Each user can own multiple labs and has a unique username.
//!
//! ## Fields
//! - `username`: Unique lower-case username, also used as the guest login name on lab nodes
//! - `password_hash`: Argon2id password hash for authentication
//! - `is_admin`: Boolean flag indicating admin privileges
//! - `ssh_keys`: Array of SSH public keys for authentication
//! - `created_at`: Timestamp when user was created (set by application)
//! - `updated_at`: Timestamp of last update (set by application)
//!
//! ## Constraints
//! - Username must be 3–32 characters long
//! - Username must match pattern: `[a-z_][a-z0-9_-]*`
//! - Username must be unique across all users
//!
//! Reserved names are enforced by `shared::validators::validate_username`, not the schema,
//! because server bootstrap creates the reserved `admin` account.
//!
//! ## Built-in `admin` protection
//! - The `username` and `is_admin` field assertions refuse renaming `admin` or clearing its
//!   admin flag, and refuse any `admin` record without the flag.
//! - The `protect_admin` event throws on deleting, renaming or demoting `admin`, which aborts
//!   the whole statement, including bulk deletes.
//!
//! These guard against application bugs and ordinary queries. A database root user can still
//! remove the definitions.
//!
//! ## Computed Fields
//! - `labs`: Reverse reference to all labs owned by this user (`<~(lab FIELD user)`)
//!
//! ## Relationships
//! - One-to-many with `lab` table (user owns multiple labs)

use shared::konst::{ADMIN_PROTECTED_MESSAGE, ADMIN_USERNAME};

/// Generate the user table schema.
///
/// Creates the user table with username validation, password authentication, and SSH key storage.
///
/// # Returns
///
/// A string containing the complete SurrealDB schema definition for the user table.
///
/// # Schema Details
///
/// - **Table**: `user` (SCHEMAFULL)
/// - **Fields**:
///   - `username`: lower-case string, 3–32 characters, with pattern validation
///   - `password_hash`: string containing Argon2id hash
///   - `is_admin`: boolean flag for admin privileges (default: false)
///   - `ssh_keys`: array of strings (default: empty array)
///   - `created_at`: datetime timestamp (set by application on creation)
///   - `updated_at`: datetime timestamp (set by application on updates)
/// - **Indexes**:
///   - `unique_username`: Ensures username uniqueness
/// - **Events**:
///   - `protect_admin`: Refuses deleting, renaming or demoting the built-in `admin` account
///
pub(crate) fn generate_user_schema() -> String {
    format!(
        r#"
DEFINE TABLE OVERWRITE user SCHEMAFULL;
DEFINE FIELD OVERWRITE username ON TABLE user TYPE string
    ASSERT string::len($value) >= 3
    AND string::len($value) <= 32
    AND $value = /^[a-z_][a-z0-9_-]*$/
    AND ($before IS NONE OR $before != '{ADMIN_USERNAME}' OR $value = '{ADMIN_USERNAME}');
DEFINE FIELD OVERWRITE password_hash ON TABLE user TYPE string;
DEFINE FIELD OVERWRITE is_admin ON TABLE user TYPE bool DEFAULT false
    ASSERT $this.username != '{ADMIN_USERNAME}' OR $value = true;
DEFINE FIELD OVERWRITE ssh_keys ON TABLE user TYPE array<string> DEFAULT [];
DEFINE FIELD OVERWRITE created_at ON TABLE user TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON TABLE user TYPE datetime;

DEFINE FIELD OVERWRITE labs ON TABLE user COMPUTED <~(lab FIELD user);

DEFINE INDEX OVERWRITE unique_username
  ON TABLE user FIELDS username UNIQUE;

DEFINE EVENT OVERWRITE protect_admin ON TABLE user
    WHEN ($before.username = '{ADMIN_USERNAME}'
        AND ($event = 'DELETE' OR $after.username != '{ADMIN_USERNAME}' OR $after.is_admin != true))
    OR ($after.username = '{ADMIN_USERNAME}' AND $after.is_admin != true)
    THEN {{ THROW '{ADMIN_PROTECTED_MESSAGE}' }};
"#
    )
}
