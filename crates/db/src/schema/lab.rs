//! Lab table schema definition
//!
//! The lab table stores network lab configurations. Each lab represents
//! a collection of network nodes and links that form a virtual network topology.
//! Labs are owned by users and identified by both a unique 8-character ID
//! and a name that must be unique per user.
//!
//! ## Fields
//! - `lab_id`: 8-character unique identifier (business key)
//! - `name`: Human-readable lab name
//! - `user`: Foreign key reference to the owning user
//!
//! ## Constraints
//! - `lab_id` must be at least 1 character (validated as exactly 8 in application)
//! - `lab_id` must be globally unique
//! - `name` must be unique per user (composite unique constraint)
//!
//! ## Computed Fields
//! - `nodes`: Reverse reference to all nodes in this lab (`<~(node FIELD lab)`)
//! - `links`: Reverse reference to all links in this lab (`<~(link FIELD lab)`)
//! - `bridges`: Reverse reference to all bridges in this lab (`<~(bridge FIELD lab)`)
//!
//! ## Relationships
//! - Many-to-one with `user` table (each lab has one owner)
//! - One-to-many with `node` table (lab contains multiple nodes)
//! - One-to-many with `link` table (lab contains multiple links between nodes)
//!
//! ## Cascade Deletion
//! The `user` field uses `REFERENCE ON DELETE CASCADE` so that when a user is
//! deleted, all their labs are automatically deleted by the database.

/// Generate the lab table schema.
///
/// Creates the lab table with unique constraints on both the lab_id (business key)
/// and the (name, user) combination to ensure labs are uniquely identifiable
/// and users cannot create duplicate lab names.
///
/// # Returns
///
/// A string containing the complete SurrealDB schema definition for the lab table.
///
/// # Schema Details
///
/// - **Table**: `lab` (SCHEMAFULL)
/// - **Fields**:
///   - `lab_id`: string with minimum length validation
///   - `name`: string (lab name)
///   - `user`: record reference to user table
/// - **Indexes**:
///   - `unique_lab_id`: Ensures lab_id uniqueness (business key)
///   - `unique_lab_name_user`: Ensures name is unique per user
///
/// # Cascade Deletion
///
/// The `user` field uses `REFERENCE ON DELETE CASCADE` so that when a user
/// is deleted, all their labs are automatically removed by the database.
///
/// # Examples
///
/// ```ignore
/// let schema = generate_lab_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_lab_schema() -> String {
    r#"
DEFINE TABLE lab SCHEMAFULL;
DEFINE FIELD lab_id ON TABLE lab TYPE string
    ASSERT string::len($value) >= 1;
DEFINE FIELD name ON TABLE lab TYPE string;
DEFINE FIELD user ON TABLE lab TYPE record<user> REFERENCE ON DELETE CASCADE;
DEFINE FIELD loopback_network ON TABLE lab TYPE string;

DEFINE FIELD nodes ON TABLE lab COMPUTED <~(node FIELD lab);
DEFINE FIELD links ON TABLE lab COMPUTED <~(link FIELD lab);
DEFINE FIELD bridges ON TABLE lab COMPUTED <~(bridge FIELD lab);

DEFINE INDEX unique_lab_id ON TABLE lab FIELDS lab_id UNIQUE;

DEFINE INDEX unique_lab_name_user
  ON TABLE lab FIELDS name, user UNIQUE;
"#
    .to_string()
}
