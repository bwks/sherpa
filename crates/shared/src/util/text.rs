use anyhow::{Result, anyhow};

pub fn split_node_int(text: &str) -> Result<(String, String)> {
    let (node, interface) = text
        .split_once("::")
        .ok_or_else(|| anyhow!("Missing :: in {}", text))?;

    Ok((node.to_string(), interface.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_node_int_valid() {
        let (node, iface) = split_node_int("router1::eth1").unwrap();
        assert_eq!(node, "router1");
        assert_eq!(iface, "eth1");
    }

    #[test]
    fn test_split_node_int_complex_interface() {
        let (node, iface) = split_node_int("sw01::GigabitEthernet0/0/1").unwrap();
        assert_eq!(node, "sw01");
        assert_eq!(iface, "GigabitEthernet0/0/1");
    }

    #[test]
    fn test_split_node_int_missing_separator() {
        let result = split_node_int("router1-eth1");
        assert!(result.is_err());
    }

    #[test]
    fn test_split_node_int_empty_string() {
        let result = split_node_int("");
        assert!(result.is_err());
    }
}
