//! Schema helper utilities
//!
//! This module provides utility functions used across multiple schema definitions.

/// Generate a comma-separated list of quoted enum values for schema ASSERT clauses.
///
/// Takes a vector of enum values (that implement Display) and converts them into
/// a format suitable for SurrealDB ASSERT IN clauses.
///
/// # Example
///
/// ```ignore
/// let bridge_kinds = vec_to_str(BridgeKind::to_vec());
/// // Returns: "ovs", "linux"
/// ```
///
/// # Parameters
///
/// * `vec` - A vector of values that implement Display (typically enums)
///
/// # Returns
///
/// A comma-separated string of quoted values, e.g., `"value1", "value2", "value3"`
pub(crate) fn vec_to_str<T: std::fmt::Display>(vec: Vec<T>) -> String {
    vec.iter()
        .map(|variant| format!(r#""{}""#, variant))
        .collect::<Vec<_>>()
        .join(", ")
}
