//! Bridge table schema definition
//!
//! The bridge table stores multi-host shared bridge definitions for labs.
//! Each bridge represents a shared network segment that multiple nodes can connect to,
//! similar to a physical switch connecting multiple devices.

/// Generate the bridge table schema.
pub(crate) fn generate_bridge_schema() -> String {
    r#"
DEFINE TABLE OVERWRITE bridge SCHEMAFULL;
DEFINE FIELD OVERWRITE index ON TABLE bridge TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD OVERWRITE bridge_name ON TABLE bridge TYPE string;
DEFINE FIELD OVERWRITE network_name ON TABLE bridge TYPE string;
DEFINE FIELD OVERWRITE lab ON TABLE bridge TYPE record<lab> REFERENCE ON DELETE CASCADE;
DEFINE FIELD OVERWRITE nodes ON TABLE bridge TYPE array<record<node>> REFERENCE ON DELETE UNSET;

DEFINE INDEX OVERWRITE unique_bridge_index
  ON TABLE bridge FIELDS index, lab UNIQUE;
"#
    .to_string()
}
