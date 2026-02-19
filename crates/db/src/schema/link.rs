//! Link table schema definition
//!
//! The link table stores network connections between nodes in a lab.
//! Each link represents a virtual network cable connecting two interfaces
//! on different nodes, including the bridge and veth pair details needed
//! for the virtual network infrastructure.
//!
//! ## Fields
//! - `index`: Link index for ordering (0-65535)
//! - `node_a`, `node_b`: Foreign key references to the two connected nodes
//! - `int_a`, `int_b`: Interface names on each node
//! - `bridge_a`, `bridge_b`: Linux bridge names on the host
//! - `veth_a`, `veth_b`: Virtual ethernet pair names
//! - `kind`: Bridge type (enum: OVS or Linux)
//! - `lab`: Foreign key reference to the owning lab
//!
//! ## Constraints
//! - Link index must be between 0 and 65535 (inclusive)
//! - Link index must be an integer (no decimals)
//! - Bridge kind must be a valid enum value (ovs, linux)
//! - Each (node_a, node_b, int_a, int_b) combination must be unique
//!
//! ## Relationships
//! - Many-to-one with `node` table (link connects two nodes)
//! - Many-to-one with `lab` table (each link belongs to one lab)

use shared::data::BridgeKind;

use super::helpers::vec_to_str;

/// Generate the link table schema with enum-driven constraints.
///
/// Creates the link table with validation rules for bridge types derived
/// from the BridgeKind enum. Ensures links have unique node/interface
/// combinations to prevent duplicate connections.
///
/// # Returns
///
/// A string containing the complete SurrealDB schema definition for the link table.
///
/// # Schema Details
///
/// - **Table**: `link` (SCHEMAFULL)
/// - **Fields**:
///   - `index`: number (0-65535, integer only)
///   - `node_a`, `node_b`: record references to node table
///   - `int_a`, `int_b`: strings (interface names)
///   - `bridge_a`, `bridge_b`: strings (bridge names)
///   - `veth_a`, `veth_b`: strings (veth pair names)
///   - `kind`: string (validated against BridgeKind enum)
///   - `lab`: record reference to lab table
/// - **Indexes**:
///   - `unique_peers_on_link`: Ensures unique (node_a, node_b, int_a, int_b) combinations
///
/// # Enum-Driven Validation
///
/// The `kind` field is validated against the BridgeKind enum:
/// - `ovs`: Open vSwitch bridge
/// - `linux`: Linux bridge
///
/// # Cascade Deletion
///
/// The `node_a`, `node_b`, and `lab` fields use `REFERENCE ON DELETE CASCADE`
/// so that when a node or lab is deleted, all associated links are automatically
/// removed by the database.
///
/// # Examples
///
/// ```ignore
/// let schema = generate_link_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_link_schema() -> String {
    let bridge_kinds = vec_to_str(BridgeKind::to_vec());

    format!(
        r#"
DEFINE TABLE link SCHEMAFULL;
DEFINE FIELD index ON TABLE link TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD node_a ON TABLE link TYPE record<node> REFERENCE ON DELETE CASCADE;
DEFINE FIELD node_b ON TABLE link TYPE record<node> REFERENCE ON DELETE CASCADE;
DEFINE FIELD int_a ON TABLE link TYPE string;
DEFINE FIELD int_b ON TABLE link TYPE string;
DEFINE FIELD bridge_a ON TABLE link TYPE string;
DEFINE FIELD bridge_b ON TABLE link TYPE string;
DEFINE FIELD veth_a ON TABLE link TYPE string;
DEFINE FIELD veth_b ON TABLE link TYPE string;
DEFINE FIELD kind ON TABLE link TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD lab ON TABLE link TYPE record<lab> REFERENCE ON DELETE CASCADE;

DEFINE INDEX unique_peers_on_link
  ON TABLE link FIELDS node_a, node_b, int_a, int_b UNIQUE;
"#,
        bridge_kinds
    )
}
