//! User table schema definition
//!
//! The user table stores user accounts with SSH keys for authentication.
//! Each user can own multiple labs and has a unique username.
//!
//! ## Fields
//! - `username`: Unique username (min 3 chars, alphanumeric + @._-)
//! - `ssh_keys`: Array of SSH public keys for authentication
//!
//! ## Constraints
//! - Username must be at least 3 characters long
//! - Username must match pattern: `[a-zA-Z0-9@._-]+`
//! - Username must be unique across all users
//!
//! ## Relationships
//! - One-to-many with `lab` table (user owns multiple labs)

/// Generate the user table schema.
///
/// Creates the user table with username validation and SSH key storage.
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
///   - `ssh_keys`: array of strings (default: empty array)
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
DEFINE FIELD ssh_keys ON TABLE user TYPE array<string> DEFAULT [];

DEFINE INDEX unique_username
  ON TABLE user FIELDS username UNIQUE;
"#
    .to_string()
}
