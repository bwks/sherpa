use anyhow::{Result, anyhow};

pub fn split_node_int(text: &String) -> Result<(String, String)> {
    let (node, interface) = text
        .split_once("::")
        .ok_or_else(|| anyhow!("Missing :: in {}", text))?;

    Ok((node.to_string(), interface.to_string()))
}
