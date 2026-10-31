//! Bridge table schema definition
//!
//! The bridge table stores multi-host shared bridge definitions for labs.
//! Each bridge represents a shared network segment that multiple nodes can connect to,
//! similar to a physical switch connecting multiple devices.
//!
//! `members` records which interface of each node is connected. It replaced a `nodes` field,
//! which held only node references with `REFERENCE ON DELETE UNSET`. SurrealDB failed to delete
//! several of a bridge's nodes in one statement through that reference, so lab destroy failed.
//! The schema removes `nodes` from existing databases: the reference has to be kept while the
//! data is unset, or the stale reference keeps rejecting node deletes.

/// Generate the bridge table schema.
pub(crate) fn generate_bridge_schema() -> String {
    r#"
DEFINE TABLE OVERWRITE bridge SCHEMAFULL;
DEFINE FIELD OVERWRITE index ON TABLE bridge TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD OVERWRITE bridge_name ON TABLE bridge TYPE string;
DEFINE FIELD OVERWRITE network_name ON TABLE bridge TYPE string;
DEFINE FIELD OVERWRITE lab ON TABLE bridge TYPE record<lab> REFERENCE ON DELETE CASCADE;
DEFINE FIELD OVERWRITE nodes ON TABLE bridge TYPE option<array<record<node>>>
    REFERENCE ON DELETE UNSET;
UPDATE bridge UNSET nodes WHERE nodes IS NOT NONE;
REMOVE FIELD IF EXISTS nodes ON TABLE bridge;
DEFINE FIELD OVERWRITE members ON TABLE bridge TYPE array<object> DEFAULT [];
DEFINE FIELD OVERWRITE members[*].node ON TABLE bridge TYPE record<node>;
DEFINE FIELD OVERWRITE members[*].interface ON TABLE bridge TYPE string;
UPDATE bridge SET members = [] WHERE members IS NONE;

DEFINE INDEX OVERWRITE unique_bridge_index
  ON TABLE bridge FIELDS index, lab UNIQUE;
"#
    .to_string()
}
