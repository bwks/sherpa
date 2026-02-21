//! Node table schema definition
//!
//! The node table stores individual network devices (nodes) within a lab.
//! Each node represents a virtual machine, container, or unikernel that
//! participates in the network topology. Nodes reference an imported image
//! from the node_image table and belong to a specific lab.
//!
//! ## Fields
//! - `name`: Node name (unique within lab)
//! - `index`: Node index for ordering (unique within lab, 0-65535)
//! - `image`: Foreign key reference to node_image record
//! - `lab`: Foreign key reference to owning lab
//! - `mgmt_ipv4`: Management IPv4 address (optional, set during lab setup)
//!
//! ## Constraints
//! - Node name must be unique per lab
//! - Node index must be unique per lab
//! - Index must be between 0 and 65535 (inclusive)
//! - Index must be an integer (no decimals)
//!
//! ## Computed Fields
//! - `links`: Reverse reference to all links connected to this node (`array::union(<~(link FIELD node_a), <~(link FIELD node_b))`)
//! - `bridges`: Reverse reference to all bridges this node connects to (`<~(bridge FIELD nodes)`)
//!
//! ## Relationships
//! - Many-to-one with `node_image` table (multiple nodes can use same image)
//! - Many-to-one with `lab` table (each node belongs to one lab)
//! - One-to-many with `link` table (node can have multiple links to other nodes)
//!
//! ## Referential Integrity
//! - `image` uses `REFERENCE ON DELETE REJECT` to prevent deletion of a
//!   node_image that is still referenced by nodes.
//! - `lab` uses `REFERENCE ON DELETE CASCADE` so that when a lab is deleted,
//!   all its nodes are automatically deleted by the database.

use shared::data::NodeState;

use super::helpers::vec_to_str;

/// Generate the node table schema.
///
/// Creates the node table with unique constraints to ensure nodes within a lab
/// have unique names and indexes. Each node references an imported image
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
///   - `image`: record reference to node_image table
///   - `lab`: record reference to lab table
///   - `mgmt_ipv4`: optional string (management IPv4 address)
/// - **Indexes**:
///   - `unique_node_name_per_lab`: Ensures node names are unique within each lab
///   - `unique_node_index_per_lab`: Ensures node indexes are unique within each lab
///
/// # Referential Integrity
///
/// - `image` uses `REFERENCE ON DELETE REJECT` to prevent deletion of a
///   node_image that is still referenced by nodes.
/// - `lab` uses `REFERENCE ON DELETE CASCADE` so that when a lab is deleted,
///   all its nodes are automatically removed by the database.
///
/// # Examples
///
/// ```ignore
/// let schema = generate_node_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_node_schema() -> String {
    let node_states = vec_to_str(NodeState::to_vec());

    format!(
        r#"
DEFINE TABLE node SCHEMAFULL;
DEFINE FIELD name ON TABLE node TYPE string;
DEFINE FIELD index ON TABLE node TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD image ON TABLE node TYPE record<node_image> REFERENCE ON DELETE REJECT;
DEFINE FIELD lab ON TABLE node TYPE record<lab> REFERENCE ON DELETE CASCADE;
DEFINE FIELD mgmt_ipv4 ON TABLE node TYPE option<string>;
DEFINE FIELD state ON TABLE node TYPE string
    ASSERT $value IN [{node_states}]
    DEFAULT "unknown";

DEFINE FIELD links ON TABLE node COMPUTED array::union(<~(link FIELD node_a), <~(link FIELD node_b));
DEFINE FIELD bridges ON TABLE node COMPUTED <~(bridge FIELD nodes);

DEFINE INDEX unique_node_name_per_lab
  ON TABLE node FIELDS lab, name UNIQUE;

DEFINE INDEX unique_node_index_per_lab
  ON TABLE node FIELDS lab, index UNIQUE;
"#
    )
}
