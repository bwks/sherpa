use anyhow::{Result, anyhow};

/// Split a `node::interface` link end into its node and interface names.
///
/// # Errors
///
/// Fails when `text` is not `node::interface`.
pub fn split_node_int(text: &str) -> Result<(String, String)> {
    let (node, interface) = text
        .split_once("::")
        .ok_or_else(|| anyhow!("Missing :: in {text}"))?;

    Ok((node.to_string(), interface.to_string()))
}
