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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_links_004_an_end_that_is_not_written_node_interface_is_refused() {
        let result = split_node_int("router1-eth1");
        assert!(result.is_err());
    }
}
