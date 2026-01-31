//! Node table schema definition
//!
//! The node table stores individual network devices (nodes) within a lab.
//! Each node represents a virtual machine, container, or unikernel that
//! participates in the network topology. Nodes are configured based on
//! templates from the node_config table and belong to a specific lab.
//!
//! ## Fields
//! - `name`: Node name (unique within lab)
//! - `index`: Node index for ordering (unique within lab, 0-65535)
//! - `config`: Foreign key reference to node_config template
//! - `lab`: Foreign key reference to owning lab
//!
//! ## Constraints
//! - Node name must be unique per lab
//! - Node index must be unique per lab
//! - Index must be between 0 and 65535 (inclusive)
//! - Index must be an integer (no decimals)
//!
//! ## Relationships
//! - Many-to-one with `node_config` table (multiple nodes can use same config)
//! - Many-to-one with `lab` table (each node belongs to one lab)
//! - One-to-many with `link` table (node can have multiple links to other nodes)

/// Generate the node table schema.
///
/// Creates the node table with unique constraints to ensure nodes within a lab
/// have unique names and indexes. Each node references a configuration template
/// and belongs to a specific lab.
///
/// # Returns
///
/// A string containing the complete SurrealDB schema definition for the node table.
///
/// # Schema Details
///
/// - **Table**: `node` (SCHEMAFULL)
/// - **Fields**:
///   - `name`: string (node name)
///   - `index`: number (0-65535, integer only)
///   - `config`: record reference to node_config table
///   - `lab`: record reference to lab table
/// - **Indexes**:
///   - `unique_node_name_per_lab`: Ensures node names are unique within each lab
///   - `unique_node_index_per_lab`: Ensures node indexes are unique within each lab
///
/// # Cascade Deletion
///
/// Note: CASCADE DELETE is commented out in the schema (SurrealDB 2.4 limitation).
/// The application handles cascade deletion manually:
/// - When a lab is deleted, all its nodes must be explicitly deleted first
/// - When a node_config is deleted, referential integrity is not enforced
///
/// # Examples
///
/// ```ignore
/// let schema = generate_node_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_node_schema() -> String {
    r#"
DEFINE TABLE node SCHEMAFULL;
DEFINE FIELD name ON TABLE node TYPE string;
DEFINE FIELD index ON TABLE node TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD config ON TABLE node TYPE record<node_config>;
DEFINE FIELD lab ON TABLE node TYPE record<lab>;
    // ON DELETE CASCADE;

DEFINE INDEX unique_node_name_per_lab
  ON TABLE node FIELDS lab, name UNIQUE;

DEFINE INDEX unique_node_index_per_lab
  ON TABLE node FIELDS lab, index UNIQUE;
"#
    .to_string()
}
