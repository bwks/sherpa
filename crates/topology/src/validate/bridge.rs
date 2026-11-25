use std::collections::HashSet;

use anyhow::{Result, bail};

use super::names::check_node_name;
use crate::{Bridge, TopologyError};

/// The fewest links a bridge joins.
const BRIDGE_MIN_LINKS: usize = 2;

/// Check every bridge has a name by the node name rule, used by no other bridge, and joins at
/// least two interfaces.
///
/// # Errors
///
/// Fails on the first bridge whose name breaks the node name rule, that has fewer than two
/// links, or whose name an earlier bridge has.
pub fn check_bridges(bridges: &[Bridge]) -> Result<()> {
    let mut names = HashSet::new();
    for bridge in bridges {
        check_node_name(&bridge.name).map_err(|reason| TopologyError::BridgeNameNotAllowed {
            bridge: bridge.name.clone(),
            reason: Box::new(reason),
        })?;
        if bridge.links.len() < BRIDGE_MIN_LINKS {
            bail!(TopologyError::BridgeTooFewLinks {
                bridge: bridge.name.clone(),
                links: bridge.links.len(),
            });
        }
        if !names.insert(bridge.name.as_str()) {
            bail!(TopologyError::DuplicateBridge {
                bridge: bridge.name.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(name: &str, links: &[&str]) -> Bridge {
        Bridge {
            name: name.to_owned(),
            links: links.iter().map(|&link| link.to_owned()).collect(),
        }
    }

    fn refusal(bridges: &[Bridge]) -> TopologyError {
        let error = check_bridges(bridges).expect_err("the bridges should be refused");
        error
            .downcast_ref::<TopologyError>()
            .cloned()
            .unwrap_or_else(|| panic!("not a topology error: {error:#}"))
    }

    #[test]
    fn manifest_links_016_a_bridge_with_no_link_or_one_link_is_refused_naming_it_and_its_count() {
        for links in [&[][..], &["r1::eth1"][..]] {
            assert_eq!(
                refusal(&[
                    bridge("br1", &["r1::eth2", "r2::eth2"]),
                    bridge("br2", links)
                ]),
                TopologyError::BridgeTooFewLinks {
                    bridge: "br2".to_owned(),
                    links: links.len(),
                }
            );
        }
        assert!(check_bridges(&[bridge("br1", &["r1::eth1", "r2::eth1"])]).is_ok());
    }

    #[test]
    fn manifest_links_017_two_bridges_with_one_name_are_refused_naming_it() {
        assert_eq!(
            refusal(&[
                bridge("br1", &["r1::eth1", "r2::eth1"]),
                bridge("br1", &["r1::eth2", "r2::eth2"]),
            ]),
            TopologyError::DuplicateBridge {
                bridge: "br1".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_links_018_a_bridge_name_the_node_name_rule_refuses_is_refused_naming_it() {
        let too_long = "b".repeat(64);
        for name in ["", "-br1", "br 1", "br/1", "br.1", "'br1'", &too_long] {
            assert_eq!(
                refusal(&[bridge(name, &["r1::eth1", "r2::eth1"])]),
                TopologyError::BridgeNameNotAllowed {
                    bridge: name.to_owned(),
                    reason: Box::new(TopologyError::NameNotPlain {
                        what: "node",
                        name: name.to_owned(),
                        max_len: 63,
                    }),
                }
            );
        }
        assert_eq!(
            refusal(&[bridge("Supervisor-br", &["r1::eth1", "r2::eth1"])]),
            TopologyError::BridgeNameNotAllowed {
                bridge: "Supervisor-br".to_owned(),
                reason: Box::new(TopologyError::ReservedNodeName {
                    name: "Supervisor-br".to_owned(),
                    word: "supervisor",
                }),
            }
        );
        assert!(check_bridges(&[bridge("Core_br-1", &["r1::eth1", "r2::eth1"])]).is_ok());
    }
}
