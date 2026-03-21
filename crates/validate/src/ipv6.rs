use std::net::Ipv6Addr;

use anyhow::{Result, bail};

use topology::Node;

/// Validate IPv6 addresses in manifest nodes.
///
/// Checks that any user-supplied `ipv6_address` values are valid unicast addresses
/// (not multicast, loopback, or unspecified).
pub fn validate_manifest_ipv6_addresses(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        if let Some(addr) = node.ipv6_address {
            validate_ipv6_address(&addr, &node.name)?;
        }
    }
    Ok(())
}

fn validate_ipv6_address(addr: &Ipv6Addr, node_name: &str) -> Result<()> {
    if addr.is_unspecified() {
        bail!(
            "Node '{}': IPv6 address '{}' is the unspecified address (::)",
            node_name,
            addr
        );
    }
    if addr.is_loopback() {
        bail!(
            "Node '{}': IPv6 address '{}' is a loopback address",
            node_name,
            addr
        );
    }
    if addr.is_multicast() {
        bail!(
            "Node '{}': IPv6 address '{}' is a multicast address",
            node_name,
            addr
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ipv6() {
        let nodes = vec![Node {
            name: "router1".to_string(),
            ipv6_address: Some("fd00:b00b:0:1::a".parse().unwrap()),
            ..Default::default()
        }];
        assert!(validate_manifest_ipv6_addresses(&nodes).is_ok());
    }

    #[test]
    fn test_no_ipv6_is_valid() {
        let nodes = vec![Node {
            name: "router1".to_string(),
            ipv6_address: None,
            ..Default::default()
        }];
        assert!(validate_manifest_ipv6_addresses(&nodes).is_ok());
    }

    #[test]
    fn test_loopback_rejected() {
        let nodes = vec![Node {
            name: "router1".to_string(),
            ipv6_address: Some("::1".parse().unwrap()),
            ..Default::default()
        }];
        let err = validate_manifest_ipv6_addresses(&nodes).unwrap_err();
        assert!(err.to_string().contains("loopback"));
    }

    #[test]
    fn test_unspecified_rejected() {
        let nodes = vec![Node {
            name: "router1".to_string(),
            ipv6_address: Some("::".parse().unwrap()),
            ..Default::default()
        }];
        let err = validate_manifest_ipv6_addresses(&nodes).unwrap_err();
        assert!(err.to_string().contains("unspecified"));
    }

    #[test]
    fn test_multicast_rejected() {
        let nodes = vec![Node {
            name: "router1".to_string(),
            ipv6_address: Some("ff02::1".parse().unwrap()),
            ..Default::default()
        }];
        let err = validate_manifest_ipv6_addresses(&nodes).unwrap_err();
        assert!(err.to_string().contains("multicast"));
    }

    #[test]
    fn test_multi_node_first_invalid_triggers_error() {
        let nodes = vec![
            Node {
                name: "good1".to_string(),
                ipv6_address: Some("fd00::1".parse().unwrap()),
                ..Default::default()
            },
            Node {
                name: "bad1".to_string(),
                ipv6_address: Some("::1".parse().unwrap()),
                ..Default::default()
            },
            Node {
                name: "good2".to_string(),
                ipv6_address: Some("fd00::2".parse().unwrap()),
                ..Default::default()
            },
        ];
        let err = validate_manifest_ipv6_addresses(&nodes).unwrap_err();
        assert!(err.to_string().contains("bad1"));
        assert!(err.to_string().contains("loopback"));
    }

    #[test]
    fn test_error_message_contains_node_name_and_address() {
        let nodes = vec![Node {
            name: "switch42".to_string(),
            ipv6_address: Some("::".parse().unwrap()),
            ..Default::default()
        }];
        let err = validate_manifest_ipv6_addresses(&nodes).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("switch42"));
        assert!(msg.contains("::"));
    }

    #[test]
    fn test_multiple_nodes_all_valid() {
        let nodes = vec![
            Node {
                name: "r1".to_string(),
                ipv6_address: Some("fd00::1".parse().unwrap()),
                ..Default::default()
            },
            Node {
                name: "r2".to_string(),
                ipv6_address: None,
                ..Default::default()
            },
            Node {
                name: "r3".to_string(),
                ipv6_address: Some("2001:db8::1".parse().unwrap()),
                ..Default::default()
            },
        ];
        assert!(validate_manifest_ipv6_addresses(&nodes).is_ok());
    }
}
