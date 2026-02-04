//! Bridge table schema definition
//!
//! The bridge table stores multi-host shared bridge definitions for labs.
//! Each bridge represents a shared network segment that multiple nodes can connect to,
//! similar to a physical switch connecting multiple devices.

/// Generate the bridge table schema.
pub(crate) fn generate_bridge_schema() -> String {
    r#"
DEFINE TABLE bridge SCHEMAFULL;
DEFINE FIELD index ON TABLE bridge TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD bridge_name ON TABLE bridge TYPE string;
DEFINE FIELD network_name ON TABLE bridge TYPE string;
DEFINE FIELD lab ON TABLE bridge TYPE record<lab>;
DEFINE FIELD nodes ON TABLE bridge TYPE array<record<node>>;

DEFINE INDEX unique_bridge_index
  ON TABLE bridge FIELDS index, lab UNIQUE;
"#
    .to_string()
}
