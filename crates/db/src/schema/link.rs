//! Link table schema definition
//!
//! The link table stores network connections between nodes in a lab.
//! Each link represents a virtual network cable connecting two interfaces
//! on different nodes, and the host ends that carry it.
//!
//! ## Fields
//! - `index`: Link index for ordering (0-65535)
//! - `node_a`, `node_b`: Foreign key references to the two connected nodes
//! - `int_a`, `int_b`: Interface names on each node
//! - `tap_a`, `tap_b`: Host end names of a P2p link
//! - `impairment_a_to_b`, `impairment_b_to_a`: netem impairment for each direction
//! - `kind`: Link type (enum: BridgeKind)
//! - `lab`: Foreign key reference to the owning lab
//!
//! Impairment was once one set of fields (`delay_us`, `jitter_us`, `loss_percent`,
//! `reorder_percent`, `corrupt_percent`) applied to both directions. The schema copies it into
//! each direction on existing links, then removes the old fields.
//!
//! Every update checks the whole record, so a field left on a record without a definition
//! fails every later update of it, and a required field cannot be unset. The schema therefore
//! redefines each old field as optional, unsets it, and only then removes its definition.
//!
//! Every link once stored `bridge_a`, `bridge_b`, `veth_a` and `veth_b`, which no link uses.
//! The schema removes them from existing links.
//!
//! ## Constraints
//! - Link index must be between 0 and 65535 (inclusive)
//! - Link index must be an integer (no decimals)
//! - Link kind must be a valid BridgeKind value
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
///   - `tap_a`, `tap_b`: strings (host end names)
///   - `kind`: string (validated against BridgeKind enum)
///   - `lab`: record reference to lab table
/// - **Indexes**:
///   - `unique_peers_on_link`: Ensures unique (node_a, node_b, int_a, int_b) combinations
///
/// # Enum-Driven Validation
///
/// The `kind` field is validated against the BridgeKind enum.
///
/// # Cascade Deletion
///
/// The `node_a`, `node_b`, and `lab` fields use `REFERENCE ON DELETE CASCADE`
/// so that when a node or lab is deleted, all associated links are automatically
/// removed by the database.
///
pub(crate) fn generate_link_schema() -> String {
    let bridge_kinds = vec_to_str(BridgeKind::to_vec());

    format!(
        r#"
DEFINE TABLE OVERWRITE link SCHEMAFULL;
DEFINE FIELD OVERWRITE index ON TABLE link TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD OVERWRITE node_a ON TABLE link TYPE record<node> REFERENCE ON DELETE CASCADE;
DEFINE FIELD OVERWRITE node_b ON TABLE link TYPE record<node> REFERENCE ON DELETE CASCADE;
DEFINE FIELD OVERWRITE int_a ON TABLE link TYPE string;
DEFINE FIELD OVERWRITE int_b ON TABLE link TYPE string;
DEFINE FIELD OVERWRITE tap_a ON TABLE link TYPE string DEFAULT '';
DEFINE FIELD OVERWRITE tap_b ON TABLE link TYPE string DEFAULT '';
DEFINE FIELD OVERWRITE impairment_a_to_b ON TABLE link TYPE object DEFAULT {{}};
DEFINE FIELD OVERWRITE impairment_a_to_b.delay_us ON TABLE link TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE impairment_a_to_b.jitter_us ON TABLE link TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE impairment_a_to_b.loss_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE impairment_a_to_b.reorder_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE impairment_a_to_b.corrupt_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE impairment_b_to_a ON TABLE link TYPE object DEFAULT {{}};
DEFINE FIELD OVERWRITE impairment_b_to_a.delay_us ON TABLE link TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE impairment_b_to_a.jitter_us ON TABLE link TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE impairment_b_to_a.loss_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE impairment_b_to_a.reorder_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE impairment_b_to_a.corrupt_percent ON TABLE link TYPE float DEFAULT 0.0;
DEFINE FIELD OVERWRITE kind ON TABLE link TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD OVERWRITE lab ON TABLE link TYPE record<lab> REFERENCE ON DELETE CASCADE;

DEFINE INDEX OVERWRITE unique_peers_on_link
  ON TABLE link FIELDS node_a, node_b, int_a, int_b UNIQUE;

DEFINE FIELD OVERWRITE delay_us ON TABLE link TYPE option<number>;
DEFINE FIELD OVERWRITE jitter_us ON TABLE link TYPE option<number>;
DEFINE FIELD OVERWRITE loss_percent ON TABLE link TYPE option<number>;
DEFINE FIELD OVERWRITE reorder_percent ON TABLE link TYPE option<number>;
DEFINE FIELD OVERWRITE corrupt_percent ON TABLE link TYPE option<number>;
DEFINE FIELD OVERWRITE bridge_a ON TABLE link TYPE option<string>;
DEFINE FIELD OVERWRITE bridge_b ON TABLE link TYPE option<string>;
DEFINE FIELD OVERWRITE veth_a ON TABLE link TYPE option<string>;
DEFINE FIELD OVERWRITE veth_b ON TABLE link TYPE option<string>;
UPDATE link SET
    impairment_a_to_b = {{
        delay_us: <int> (delay_us ?? 0), jitter_us: <int> (jitter_us ?? 0),
        loss_percent: <float> (loss_percent ?? 0), reorder_percent: <float> (reorder_percent ?? 0),
        corrupt_percent: <float> (corrupt_percent ?? 0)
    }},
    impairment_b_to_a = {{
        delay_us: <int> (delay_us ?? 0), jitter_us: <int> (jitter_us ?? 0),
        loss_percent: <float> (loss_percent ?? 0), reorder_percent: <float> (reorder_percent ?? 0),
        corrupt_percent: <float> (corrupt_percent ?? 0)
    }}
    WHERE delay_us IS NOT NONE;
UPDATE link UNSET delay_us, jitter_us, loss_percent, reorder_percent, corrupt_percent
    WHERE delay_us IS NOT NONE;
REMOVE FIELD IF EXISTS delay_us ON TABLE link;
REMOVE FIELD IF EXISTS jitter_us ON TABLE link;
REMOVE FIELD IF EXISTS loss_percent ON TABLE link;
REMOVE FIELD IF EXISTS reorder_percent ON TABLE link;
REMOVE FIELD IF EXISTS corrupt_percent ON TABLE link;
UPDATE link UNSET bridge_a, bridge_b, veth_a, veth_b WHERE bridge_a IS NOT NONE;
REMOVE FIELD IF EXISTS bridge_a ON TABLE link;
REMOVE FIELD IF EXISTS bridge_b ON TABLE link;
REMOVE FIELD IF EXISTS veth_a ON TABLE link;
REMOVE FIELD IF EXISTS veth_b ON TABLE link;
"#,
        bridge_kinds
    )
}
