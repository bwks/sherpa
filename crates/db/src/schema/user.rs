//! User table schema definition
//!
//! The user table stores user accounts with password authentication and optional SSH keys.
//! Each user can own multiple labs and has a unique username.
//!
//! ## Fields
//! - `username`: Unique username (min 3 chars, alphanumeric + @._-)
//! - `password_hash`: Argon2id password hash for authentication
//! - `is_admin`: Boolean flag indicating admin privileges
//! - `ssh_keys`: Array of SSH public keys for authentication
//! - `created_at`: Timestamp when user was created (set by application)
//! - `updated_at`: Timestamp of last update (set by application)
//!
//! ## Constraints
//! - Username must be at least 3 characters long
//! - Username must match pattern: `[a-zA-Z0-9@._-]+`
//! - Username must be unique across all users
//!
//! ## Computed Fields
//! - `labs`: Reverse reference to all labs owned by this user (`<~(lab FIELD user)`)
//!
//! ## Relationships
//! - One-to-many with `lab` table (user owns multiple labs)

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
///   - `username`: string with length >= 3 and pattern validation
///   - `password_hash`: string containing Argon2id hash
///   - `is_admin`: boolean flag for admin privileges (default: false)
///   - `ssh_keys`: array of strings (default: empty array)
///   - `created_at`: datetime timestamp (set by application on creation)
///   - `updated_at`: datetime timestamp (set by application on updates)
/// - **Indexes**:
///   - `unique_username`: Ensures username uniqueness
///
/// # Examples
///
/// ```ignore
/// let schema = generate_user_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_user_schema() -> String {
    r#"
DEFINE TABLE user SCHEMAFULL;
DEFINE FIELD username ON TABLE user TYPE string
    ASSERT string::len($value) >= 3
    AND $value = /^[a-zA-Z0-9@._-]+$/;
DEFINE FIELD password_hash ON TABLE user TYPE string;
DEFINE FIELD is_admin ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD ssh_keys ON TABLE user TYPE array<string> DEFAULT [];
DEFINE FIELD created_at ON TABLE user TYPE datetime;
DEFINE FIELD updated_at ON TABLE user TYPE datetime;

DEFINE FIELD labs ON TABLE user COMPUTED <~(lab FIELD user);

DEFINE INDEX unique_username
  ON TABLE user FIELDS username UNIQUE;
"#
    .to_string()
}
