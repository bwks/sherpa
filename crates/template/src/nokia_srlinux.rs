use std::net::Ipv4Addr;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use shared::data::{Dns, NetworkV4, User};

/// Factory ACL config extracted from a clean SR Linux v25.10.2 container.
/// Includes CPM ACL filters (IPv4 + IPv6), policers, and control-plane-traffic
/// section needed to steer traffic (SSH, gNMI, etc.) to the correct namespace.
static FACTORY_ACL_JSON: &str = include_str!("nokia_srlinux_factory_acl.json");

/// SR Linux config.json uses YANG module-prefixed keys.
/// The config.json fully replaces factory defaults, so we must include
/// all referenced objects: network-instance mgmt, AAA server-group local,
/// interface mgmt0, and the system configuration.

#[derive(Serialize)]
struct SrlinuxConfig {
    #[serde(rename = "srl_nokia-system:system")]
    system: System,
    #[serde(rename = "srl_nokia-interfaces:interface")]
    interface: Vec<Interface>,
    #[serde(rename = "srl_nokia-network-instance:network-instance")]
    network_instance: Vec<NetworkInstance>,
}

// ============================================================================
// System
// ============================================================================

#[derive(Serialize)]
struct System {
    #[serde(rename = "srl_nokia-system-name:name")]
    name: SystemName,
    #[serde(rename = "srl_nokia-dns:dns")]
    dns: SystemDns,
    #[serde(rename = "srl_nokia-aaa:aaa")]
    aaa: Aaa,
    #[serde(rename = "srl_nokia-ssh:ssh-server")]
    ssh_server: Vec<SshServer>,
    #[serde(rename = "srl_nokia-lldp:lldp")]
    lldp: Lldp,
}

#[derive(Serialize)]
struct SystemName {
    #[serde(rename = "host-name")]
    host_name: String,
}

#[derive(Serialize)]
struct SystemDns {
    #[serde(rename = "network-instance")]
    network_instance: String,
    #[serde(rename = "server-list")]
    server_list: Vec<String>,
}

#[derive(Serialize)]
struct SshServer {
    name: String,
    #[serde(rename = "admin-state")]
    admin_state: String,
    #[serde(rename = "network-instance")]
    network_instance: String,
}

// ============================================================================
// LLDP
// ============================================================================

#[derive(Serialize)]
struct Lldp {
    #[serde(rename = "admin-state")]
    admin_state: String,
}

// ============================================================================
// AAA
// ============================================================================

#[derive(Serialize)]
struct Aaa {
    authentication: Authentication,
    authorization: AaaAuthorization,
    #[serde(rename = "server-group")]
    server_group: Vec<AaaServerGroup>,
}

#[derive(Serialize)]
struct AaaAuthorization {
    role: Vec<AaaRole>,
}

#[derive(Serialize)]
struct AaaRole {
    rolename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    superuser: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    services: Vec<String>,
}

#[derive(Serialize)]
struct Authentication {
    #[serde(rename = "authentication-method")]
    authentication_method: Vec<String>,
    #[serde(rename = "admin-user")]
    admin_user: AdminUser,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    user: Vec<AaaUser>,
}

#[derive(Serialize)]
struct AdminUser {
    password: String,
    #[serde(rename = "ssh-key", skip_serializing_if = "Vec::is_empty")]
    ssh_key: Vec<String>,
}

#[derive(Serialize)]
struct AaaUser {
    username: String,
    password: String,
    #[serde(rename = "ssh-key", skip_serializing_if = "Vec::is_empty")]
    ssh_key: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    role: Vec<String>,
}

#[derive(Serialize)]
struct AaaServerGroup {
    name: String,
    #[serde(rename = "type")]
    sg_type: String,
}

// ============================================================================
// Interface
// ============================================================================

#[derive(Serialize)]
struct Interface {
    name: String,
    #[serde(rename = "admin-state")]
    admin_state: String,
    subinterface: Vec<Subinterface>,
}

#[derive(Serialize)]
struct Subinterface {
    index: u32,
    #[serde(rename = "admin-state")]
    admin_state: String,
    ipv4: SubinterfaceIpv4,
    ipv6: SubinterfaceIpv6,
}

#[derive(Serialize)]
struct SubinterfaceIpv4 {
    #[serde(rename = "admin-state")]
    admin_state: String,
    #[serde(
        rename = "srl_nokia-interfaces-ip-dhcp:dhcp-client",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp_client: Option<DhcpClient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<Vec<Ipv4Address>>,
}

#[derive(Serialize)]
struct SubinterfaceIpv6 {
    #[serde(rename = "admin-state")]
    admin_state: String,
    #[serde(rename = "srl_nokia-interfaces-ip-dhcp:dhcp-client")]
    dhcp_client: DhcpClient,
}

#[derive(Serialize)]
struct DhcpClient {}

#[derive(Serialize)]
struct Ipv4Address {
    #[serde(rename = "ip-prefix")]
    ip_prefix: String,
}

// ============================================================================
// Network Instance
// ============================================================================

#[derive(Serialize)]
struct NetworkInstance {
    name: String,
    #[serde(rename = "type")]
    ni_type: String,
    #[serde(rename = "admin-state")]
    admin_state: String,
    description: String,
    interface: Vec<NiInterface>,
    protocols: NiProtocols,
}

#[derive(Serialize)]
struct NiInterface {
    name: String,
}

#[derive(Serialize)]
struct NiProtocols {
    #[serde(rename = "srl_nokia-linux:linux")]
    linux: NiLinux,
}

#[derive(Serialize)]
struct NiLinux {
    #[serde(rename = "import-routes")]
    import_routes: bool,
    #[serde(rename = "export-routes")]
    export_routes: bool,
    #[serde(rename = "export-neighbors")]
    export_neighbors: bool,
}

// ============================================================================
// Builder
// ============================================================================

pub fn build_srlinux_config(
    hostname: &str,
    user: &User,
    dns: &Dns,
    mgmt_ipv4: &NetworkV4,
    mgmt_ipv4_address: Option<Ipv4Addr>,
) -> Result<String> {
    let password = user.password.clone().unwrap_or_default();

    let ssh_key = format!(
        "{} {}",
        user.ssh_public_key.algorithm, user.ssh_public_key.key
    );

    let dns_servers: Vec<String> = dns
        .name_servers
        .iter()
        .map(|ns| ns.ipv4_address.to_string())
        .collect();

    let ipv4 = match mgmt_ipv4_address {
        Some(addr) => SubinterfaceIpv4 {
            admin_state: "enable".to_string(),
            dhcp_client: None,
            address: Some(vec![Ipv4Address {
                ip_prefix: format!("{}/{}", addr, mgmt_ipv4.prefix_length),
            }]),
        },
        None => SubinterfaceIpv4 {
            admin_state: "enable".to_string(),
            dhcp_client: Some(DhcpClient {}),
            address: None,
        },
    };

    let config = SrlinuxConfig {
        system: System {
            name: SystemName {
                host_name: hostname.to_string(),
            },
            dns: SystemDns {
                network_instance: "mgmt".to_string(),
                server_list: dns_servers,
            },
            aaa: Aaa {
                authentication: Authentication {
                    authentication_method: vec!["local".to_string()],
                    admin_user: AdminUser {
                        password: password.clone(),
                        ssh_key: vec![ssh_key.clone()],
                    },
                    user: vec![AaaUser {
                        username: user.username.clone(),
                        password,
                        ssh_key: vec![ssh_key],
                        role: vec!["srl-admin".to_string()],
                    }],
                },
                authorization: AaaAuthorization {
                    role: vec![AaaRole {
                        rolename: "srl-admin".to_string(),
                        superuser: Some(true),
                        services: vec![
                            "cli".to_string(),
                            "gnmi".to_string(),
                            "json-rpc".to_string(),
                        ],
                    }],
                },
                server_group: vec![AaaServerGroup {
                    name: "local".to_string(),
                    sg_type: "srl_nokia-aaa-types:local".to_string(),
                }],
            },
            ssh_server: vec![SshServer {
                name: "mgmt".to_string(),
                admin_state: "enable".to_string(),
                network_instance: "mgmt".to_string(),
            }],
            lldp: Lldp {
                admin_state: "enable".to_string(),
            },
        },
        interface: vec![Interface {
            name: "mgmt0".to_string(),
            admin_state: "enable".to_string(),
            subinterface: vec![Subinterface {
                index: 0,
                admin_state: "enable".to_string(),
                ipv4,
                ipv6: SubinterfaceIpv6 {
                    admin_state: "enable".to_string(),
                    dhcp_client: DhcpClient {},
                },
            }],
        }],
        network_instance: vec![NetworkInstance {
            name: "mgmt".to_string(),
            ni_type: "srl_nokia-network-instance:ip-vrf".to_string(),
            admin_state: "enable".to_string(),
            description: "Management network instance".to_string(),
            interface: vec![NiInterface {
                name: "mgmt0.0".to_string(),
            }],
            protocols: NiProtocols {
                linux: NiLinux {
                    import_routes: true,
                    export_routes: true,
                    export_neighbors: true,
                },
            },
        }],
    };

    // Serialize structured config to a Value so we can merge factory ACL config
    let mut config_value = serde_json::to_value(&config)?;
    let factory_acl: Value = serde_json::from_str(FACTORY_ACL_JSON)?;

    // Merge top-level ACL key
    if let Some(acl) = factory_acl.get("srl_nokia-acl:acl") {
        config_value["srl_nokia-acl:acl"] = acl.clone();
    }

    // Merge control-plane-traffic into system section
    if let Some(cpt) = factory_acl.get("control-plane-traffic") {
        config_value["srl_nokia-system:system"]["control-plane-traffic"] = cpt.clone();
    }

    let json = serde_json::to_string_pretty(&config_value)?;
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{SshKeyAlgorithms, SshPublicKey};
    use std::net::Ipv4Addr;

    fn test_user() -> User {
        User {
            username: "admin".to_string(),
            password: Some("admin123".to_string()),
            ssh_public_key: SshPublicKey {
                algorithm: SshKeyAlgorithms::SshRsa,
                key: "AAAAB3NzaC1yc2EAAAADAQABAAABAQ".to_string(),
                comment: Some("test".to_string()),
            },
            sudo: true,
        }
    }

    fn test_dns() -> Dns {
        use shared::data::NameServer;
        Dns {
            domain: "lab.local".to_string(),
            name_servers: vec![NameServer {
                name: "ns1".to_string(),
                ipv4_address: Ipv4Addr::new(172, 31, 0, 1),
                ipv6_address: None,
            }],
        }
    }

    fn test_network() -> NetworkV4 {
        use ipnet::Ipv4Net;
        NetworkV4 {
            prefix: "172.31.0.0/16".parse::<Ipv4Net>().expect("valid prefix"),
            first: Ipv4Addr::new(172, 31, 0, 1),
            last: Ipv4Addr::new(172, 31, 255, 254),
            boot_server: Ipv4Addr::new(172, 31, 0, 1),
            network: Ipv4Addr::new(172, 31, 0, 0),
            subnet_mask: Ipv4Addr::new(255, 255, 0, 0),
            hostmask: Ipv4Addr::new(0, 0, 255, 255),
            prefix_length: 16,
        }
    }

    #[test]
    fn test_build_srlinux_config_with_static_ip() {
        let user = test_user();
        let dns = test_dns();
        let network = test_network();
        let addr = Some(Ipv4Addr::new(172, 31, 0, 10));

        let result = build_srlinux_config("srl01", &user, &dns, &network, addr);
        assert!(result.is_ok());

        let json = result.expect("valid json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json structure");

        // System
        let system = &parsed["srl_nokia-system:system"];
        assert_eq!(system["srl_nokia-system-name:name"]["host-name"], "srl01");
        assert_eq!(system["srl_nokia-dns:dns"]["network-instance"], "mgmt");
        assert_eq!(system["srl_nokia-dns:dns"]["server-list"][0], "172.31.0.1");

        // AAA
        let aaa = &system["srl_nokia-aaa:aaa"];
        assert_eq!(aaa["authentication"]["authentication-method"][0], "local");
        assert_eq!(aaa["authentication"]["admin-user"]["password"], "admin123");
        assert_eq!(aaa["authentication"]["user"][0]["username"], "admin");
        assert_eq!(aaa["authentication"]["user"][0]["password"], "admin123");
        assert_eq!(aaa["server-group"][0]["name"], "local");
        assert_eq!(aaa["server-group"][0]["type"], "srl_nokia-aaa-types:local");

        // SSH server
        assert_eq!(system["srl_nokia-ssh:ssh-server"][0]["name"], "mgmt");

        // Interface - static IP, no DHCP
        let intf = &parsed["srl_nokia-interfaces:interface"][0];
        assert_eq!(intf["name"], "mgmt0");
        let sub = &intf["subinterface"][0];
        assert_eq!(sub["ipv4"]["address"][0]["ip-prefix"], "172.31.0.10/16");
        assert!(
            sub["ipv4"]
                .get("srl_nokia-interfaces-ip-dhcp:dhcp-client")
                .is_none()
        );

        // Network instance
        let ni = &parsed["srl_nokia-network-instance:network-instance"][0];
        assert_eq!(ni["name"], "mgmt");
        assert_eq!(ni["type"], "srl_nokia-network-instance:ip-vrf");
        assert_eq!(ni["interface"][0]["name"], "mgmt0.0");

        // ACL and control-plane-traffic (factory defaults for traffic steering)
        assert!(parsed["srl_nokia-acl:acl"]["acl-filter"].is_array());
        assert!(parsed["srl_nokia-system:system"]["control-plane-traffic"].is_object());
    }

    #[test]
    fn test_build_srlinux_config_with_dhcp() {
        let user = test_user();
        let dns = test_dns();
        let network = test_network();

        let result = build_srlinux_config("srl02", &user, &dns, &network, None);
        assert!(result.is_ok());

        let json = result.expect("valid json");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json structure");

        assert_eq!(
            parsed["srl_nokia-system:system"]["srl_nokia-system-name:name"]["host-name"],
            "srl02"
        );

        // DHCP mode
        let sub = &parsed["srl_nokia-interfaces:interface"][0]["subinterface"][0];
        assert!(sub["ipv4"]["address"].is_null());
        assert!(sub["ipv4"]["srl_nokia-interfaces-ip-dhcp:dhcp-client"].is_object());
    }
}
