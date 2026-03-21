use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use shared::data::{Dns, NetworkV4, NetworkV6, User};

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
    #[serde(
        rename = "srl_nokia-interfaces-ip-dhcp:dhcp-client",
        skip_serializing_if = "Option::is_none"
    )]
    dhcp_client: Option<DhcpClient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<Vec<Ipv6Address>>,
}

#[derive(Serialize)]
struct DhcpClient {}

#[derive(Serialize)]
struct Ipv4Address {
    #[serde(rename = "ip-prefix")]
    ip_prefix: String,
}

#[derive(Serialize)]
struct Ipv6Address {
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
    mgmt_ipv6_address: Option<Ipv6Addr>,
    mgmt_ipv6: Option<&NetworkV6>,
) -> Result<String> {
    let password = user.password.clone().unwrap_or_default();

    let ssh_key = format!(
        "{} {}",
        user.ssh_public_key.algorithm, user.ssh_public_key.key
    );

    let mut dns_servers: Vec<String> = dns
        .name_servers
        .iter()
        .map(|ns| ns.ipv4_address.to_string())
        .collect();
    for ns in &dns.name_servers {
        if let Some(ipv6) = ns.ipv6_address {
            dns_servers.push(ipv6.to_string());
        }
    }

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

    let ipv6 = match (mgmt_ipv6_address, mgmt_ipv6) {
        (Some(addr), Some(v6)) => SubinterfaceIpv6 {
            admin_state: "enable".to_string(),
            dhcp_client: None,
            address: Some(vec![Ipv6Address {
                ip_prefix: format!("{}/{}", addr, v6.prefix_length),
            }]),
        },
        _ => SubinterfaceIpv6 {
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
                ipv6,
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
