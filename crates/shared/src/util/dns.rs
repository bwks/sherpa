use crate::data::{Dns, NameServer, SherpaNetwork};
use crate::konst::SHERPA_DOMAIN_NAME;

/// The lab's DNS settings: Sherpa's domain, with the lab's boot server as name server, on IPv4
/// and, when the lab has it, IPv6. The boot server holds a name record for every node.
#[must_use]
pub fn default_dns(mgmt_net: &SherpaNetwork) -> Dns {
    Dns {
        domain: SHERPA_DOMAIN_NAME.to_owned(),
        name_servers: vec![NameServer {
            ipv4_address: mgmt_net.v4.boot_server,
            ipv6_address: mgmt_net.v6.as_ref().map(|v6| v6.boot_server),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{NetworkV4, NetworkV6};

    /// A lab's management network, its gateway `.1` and its boot server `.2`.
    fn lab_network(v6: bool) -> SherpaNetwork {
        let v4_prefix: ipnet::Ipv4Net = "172.31.1.0/24".parse().unwrap();
        let v6_prefix: ipnet::Ipv6Net = "fd00:b00b:0:1::/64".parse().unwrap();
        SherpaNetwork {
            v4: NetworkV4 {
                prefix: v4_prefix,
                first: "172.31.1.1".parse().unwrap(),
                last: v4_prefix.broadcast(),
                boot_server: "172.31.1.2".parse().unwrap(),
                network: v4_prefix.network(),
                subnet_mask: v4_prefix.netmask(),
                hostmask: v4_prefix.hostmask(),
                prefix_length: v4_prefix.prefix_len(),
            },
            v6: v6.then(|| NetworkV6 {
                prefix: v6_prefix,
                first: "fd00:b00b:0:1::1".parse().unwrap(),
                last: "fd00:b00b:0:1:ffff:ffff:ffff:ffff".parse().unwrap(),
                boot_server: "fd00:b00b:0:1::2".parse().unwrap(),
                network: v6_prefix.network(),
                prefix_length: v6_prefix.prefix_len(),
            }),
        }
    }

    #[test]
    fn config_name_server_001_the_lab_name_server_is_its_boot_server_not_its_gateway() {
        let v4_only = default_dns(&lab_network(false));
        assert_eq!(v4_only.domain, SHERPA_DOMAIN_NAME);
        assert_eq!(v4_only.name_servers.len(), 1);
        assert_eq!(
            v4_only.name_servers[0].ipv4_address.to_string(),
            "172.31.1.2"
        );
        assert_eq!(v4_only.name_servers[0].ipv6_address, None);

        let dual_stack = default_dns(&lab_network(true));
        assert_eq!(dual_stack.name_servers.len(), 1);
        assert_eq!(
            dual_stack.name_servers[0].ipv4_address.to_string(),
            "172.31.1.2"
        );
        assert_eq!(
            dual_stack.name_servers[0]
                .ipv6_address
                .map(|address| address.to_string()),
            Some("fd00:b00b:0:1::2".to_string())
        );
    }
}
