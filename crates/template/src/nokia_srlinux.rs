use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use shared::data::{Dns, OwnerUser};

/// Factory ACL config extracted from a clean SR Linux v25.10.2 container.
/// Includes CPM ACL filters (IPv4 + IPv6), policers, and control-plane-traffic
/// section needed to steer traffic (SSH, gNMI, etc.) to the correct namespace.
static FACTORY_ACL_JSON: &str = include_str!("nokia_srlinux_factory_acl.json");
/// The name of the DNS instance Sherpa configures, in the `mgmt` network instance.
const DNS_INSTANCE_NAME: &str = "mgmt";

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
    #[serde(rename = "srl_nokia-dns:dns-instance")]
    dns_instance: Vec<DnsInstance>,
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

/// A DNS client. Release 26 replaced the single `dns` container with this list.
#[derive(Serialize)]
struct DnsInstance {
    name: String,
    #[serde(rename = "network-instance")]
    network_instance: String,
    #[serde(rename = "search-list")]
    search_list: Vec<String>,
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
    method: Vec<String>,
    #[serde(rename = "admin-user")]
    admin_user: AdminUser,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    user: Vec<AaaUser>,
}

#[derive(Serialize)]
struct AdminUser {
    password: String,
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
    ipv4: SubinterfaceIp,
    ipv6: SubinterfaceIp,
}

#[derive(Serialize)]
struct SubinterfaceIp {
    #[serde(rename = "admin-state")]
    admin_state: String,
    #[serde(rename = "srl_nokia-interfaces-ip-dhcp:dhcp-client")]
    dhcp_client: DhcpClient,
}

#[derive(Serialize)]
struct DhcpClient {}

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

/// The SR Linux startup configuration, as JSON: the accounts, DNS, the management interface
/// and network instance, and the factory ACLs. The built-in `admin` takes the owner's device
/// password, and the owner is the one other account.
///
/// # Errors
///
/// Fails when the configuration or the factory ACLs cannot be serialized or parsed.
pub fn build_srlinux_config(hostname: &str, owner: &OwnerUser, dns: &Dns) -> Result<String> {
    let users = vec![owner_aaa_user(owner)];
    let dns_servers = dns_servers(dns);

    let config = SrlinuxConfig {
        system: System {
            name: SystemName {
                host_name: hostname.to_string(),
            },
            dns_instance: vec![DnsInstance {
                name: DNS_INSTANCE_NAME.to_string(),
                network_instance: "mgmt".to_string(),
                search_list: vec![dns.domain.clone()],
                server_list: dns_servers,
            }],
            aaa: Aaa {
                authentication: Authentication {
                    method: vec!["local".to_string()],
                    admin_user: AdminUser {
                        password: owner.password.clone(),
                    },
                    user: users,
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
            subinterface: vec![management_subinterface()],
        }],
        network_instance: vec![management_network_instance()],
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

/// The lab owner's account, an `srl-admin` with every key.
fn owner_aaa_user(owner: &OwnerUser) -> AaaUser {
    let mut ssh_key = Vec::new();
    for key in &owner.ssh_keys {
        ssh_key.push(format!("{} {}", key.kind, key.value));
    }
    AaaUser {
        username: owner.username.clone(),
        password: owner.password.clone(),
        ssh_key,
        role: vec!["srl-admin".to_string()],
    }
}

/// The name servers' IPv4 addresses, then their IPv6 addresses.
fn dns_servers(dns: &Dns) -> Vec<String> {
    let mut servers: Vec<String> = dns
        .name_servers
        .iter()
        .map(|ns| ns.ipv4_address.to_string())
        .collect();
    for ns in &dns.name_servers {
        if let Some(ipv6) = ns.ipv6_address {
            servers.push(ipv6.to_string());
        }
    }
    servers
}

/// `mgmt0.0`, with a DHCP client for IPv4 and for IPv6. In a container, SR Linux's DHCP client
/// takes the addresses `mgmt0` has when it starts, the ones the runtime gave `eth0`, and moves
/// them from SR Linux's base namespace to `mgmt0.0`, so the node keeps the management addresses
/// Sherpa gave it and they are on one interface only.
fn management_subinterface() -> Subinterface {
    let dhcp = || SubinterfaceIp {
        admin_state: "enable".to_string(),
        dhcp_client: DhcpClient {},
    };
    Subinterface {
        index: 0,
        admin_state: "enable".to_string(),
        ipv4: dhcp(),
        ipv6: dhcp(),
    }
}

/// The `mgmt` network instance, which holds `mgmt0.0` and shares routes with Linux.
fn management_network_instance() -> NetworkInstance {
    NetworkInstance {
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
    }
}
