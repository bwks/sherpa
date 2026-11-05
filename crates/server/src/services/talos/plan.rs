use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shared::data::{ClusterCni, ClusterRole, NetworkV4};
use shared::konst::{
    TALOS_LOAD_BALANCER_BLOCK_SIZE, TALOS_LOAD_BALANCER_HOST_OFFSETS, TALOS_VIP_HOST_OFFSETS,
};
use shared::util::get_ipv4_addr;
use topology::TalosCluster;

/// Kubernetes API port.
pub const KUBERNETES_API_PORT: u16 = 6443;

/// A node a cluster is made of or uses, and its management address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMember {
    pub name: String,
    pub ipv4: Ipv4Addr,
}

/// A manifest node's cluster membership, before planning.
#[derive(Clone, Debug)]
pub struct MemberInput<'a> {
    pub name: &'a str,
    pub cluster: &'a str,
    pub role: ClusterRole,
    pub ipv4: Ipv4Addr,
}

/// Management addresses from `first` to `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressRange {
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
}

/// Addresses and members of one cluster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPlan {
    pub name: String,
    /// Address the Kubernetes API is reached on: the shared address with several control
    /// planes, otherwise the only control plane's address.
    pub endpoint: Ipv4Addr,
    /// Shared address the control planes elect an owner for.
    pub vip: Option<Ipv4Addr>,
    /// Plans saved before clusters had a CNI choice are Flannel clusters.
    #[serde(default)]
    pub cni: ClusterCni,
    /// Addresses a Cilium cluster hands out to load balancers: its Gateway first, then Hubble
    /// UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_balancer_addresses: Option<AddressRange>,
    /// Control planes in manifest order. The first one is bootstrapped.
    pub control_planes: Vec<ClusterMember>,
    pub workers: Vec<ClusterMember>,
    /// The Keycloak node whose realm users log in to the cluster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<ClusterMember>,
    /// The Vault node the cluster's workloads read secrets from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<ClusterMember>,
}

impl ClusterPlan {
    pub fn endpoint_url(&self) -> String {
        format!("https://{}:{KUBERNETES_API_PORT}", self.endpoint)
    }

    /// With no workers, workloads must be allowed on the control planes.
    pub fn schedulable_control_planes(&self) -> bool {
        self.workers.is_empty()
    }

    /// The one control plane that receives the bootstrap request.
    pub fn bootstrap_node(&self) -> Option<&ClusterMember> {
        self.control_planes.first()
    }

    pub fn member_count(&self) -> usize {
        self.control_planes.len() + self.workers.len()
    }

    /// Address of a Cilium cluster's Gateway: the first load balancer address.
    pub fn gateway_address(&self) -> Option<Ipv4Addr> {
        self.load_balancer_addresses.map(|range| range.first)
    }

    /// Address of a Cilium cluster's Hubble UI: the load balancer address after the Gateway's.
    pub fn hubble_ui_address(&self) -> Option<Ipv4Addr> {
        self.gateway_address()
            .map(|gateway| Ipv4Addr::from(u32::from(gateway) + 1))
    }

    pub fn role_of(&self, node_name: &str) -> Option<ClusterRole> {
        if self
            .control_planes
            .iter()
            .any(|member| member.name == node_name)
        {
            Some(ClusterRole::ControlPlane)
        } else if self.workers.iter().any(|member| member.name == node_name) {
            Some(ClusterRole::Worker)
        } else {
            None
        }
    }
}

/// Plan every cluster: assign shared addresses to clusters with several control planes and
/// blocks of load balancer addresses to Cilium clusters, in manifest order, from the reserved
/// offsets in the management subnet. Clusters must already be validated.
///
/// `services` holds the addresses of the Keycloak and Vault nodes clusters use.
pub fn plan_clusters(
    clusters: &[TalosCluster],
    members: &[MemberInput],
    services: &[ClusterMember],
    management: &NetworkV4,
) -> Result<Vec<ClusterPlan>> {
    let mut vip_offsets = TALOS_VIP_HOST_OFFSETS;
    let mut load_balancer_blocks =
        TALOS_LOAD_BALANCER_HOST_OFFSETS.step_by(TALOS_LOAD_BALANCER_BLOCK_SIZE as usize);
    let mut plans = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let name = &cluster.name;
        let mut control_planes = vec![];
        let mut workers = vec![];
        for member in members.iter().filter(|member| member.cluster == name) {
            let entry = ClusterMember {
                name: member.name.to_owned(),
                ipv4: member.ipv4,
            };
            match member.role {
                ClusterRole::ControlPlane => control_planes.push(entry),
                ClusterRole::Worker => workers.push(entry),
            }
        }
        let first_control_plane = control_planes
            .first()
            .with_context(|| format!("Cluster '{name}' has no control plane"))?
            .ipv4;
        let vip = if control_planes.len() > 1 {
            let offset = vip_offsets
                .next()
                .with_context(|| format!("No shared address left for cluster '{name}'"))?;
            Some(get_ipv4_addr(&management.prefix, offset)?)
        } else {
            None
        };
        let load_balancer_addresses = match cluster.cni {
            ClusterCni::Cilium => {
                let start = load_balancer_blocks.next().with_context(|| {
                    format!("No load balancer addresses left for cluster '{name}'")
                })?;
                Some(AddressRange {
                    first: get_ipv4_addr(&management.prefix, start)?,
                    last: get_ipv4_addr(
                        &management.prefix,
                        start + TALOS_LOAD_BALANCER_BLOCK_SIZE - 1,
                    )?,
                })
            }
            ClusterCni::Flannel => None,
        };
        plans.push(ClusterPlan {
            name: name.to_owned(),
            endpoint: vip.unwrap_or(first_control_plane),
            vip,
            cni: cluster.cni,
            load_balancer_addresses,
            control_planes,
            workers,
            oidc: find_service(services, cluster.oidc.as_deref(), name)?,
            vault: find_service(services, cluster.vault.as_deref(), name)?,
        });
    }
    Ok(plans)
}

/// The address of the node a cluster uses, if it uses one.
fn find_service(
    services: &[ClusterMember],
    node_name: Option<&str>,
    cluster_name: &str,
) -> Result<Option<ClusterMember>> {
    let Some(node_name) = node_name else {
        return Ok(None);
    };
    let service = services
        .iter()
        .find(|service| service.name == node_name)
        .with_context(|| {
            format!("Node '{node_name}', used by cluster '{cluster_name}', has no address")
        })?;
    Ok(Some(service.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(name: &str, cni: ClusterCni) -> TalosCluster {
        TalosCluster {
            name: name.to_owned(),
            cni,
            ..TalosCluster::default()
        }
    }

    fn management() -> NetworkV4 {
        NetworkV4 {
            prefix: "172.31.1.0/24".parse().unwrap(),
            first: "172.31.1.1".parse().unwrap(),
            last: "172.31.1.255".parse().unwrap(),
            boot_server: "172.31.1.2".parse().unwrap(),
            network: "172.31.1.0".parse().unwrap(),
            subnet_mask: "255.255.255.0".parse().unwrap(),
            hostmask: "0.0.0.255".parse().unwrap(),
            prefix_length: 24,
        }
    }

    fn member<'a>(name: &'a str, cluster: &'a str, role: ClusterRole, host: u8) -> MemberInput<'a> {
        MemberInput {
            name,
            cluster,
            role,
            ipv4: Ipv4Addr::new(172, 31, 1, host),
        }
    }

    #[test]
    fn single_control_plane_clusters_use_its_address_and_ha_clusters_get_a_vip() {
        let members = [
            member("a-cp1", "a", ClusterRole::ControlPlane, 21),
            member("a-w1", "a", ClusterRole::Worker, 22),
            member("b-cp1", "b", ClusterRole::ControlPlane, 23),
            member("b-cp2", "b", ClusterRole::ControlPlane, 24),
            member("b-cp3", "b", ClusterRole::ControlPlane, 25),
            member("c-cp1", "c", ClusterRole::ControlPlane, 26),
            member("c-cp2", "c", ClusterRole::ControlPlane, 27),
            member("c-cp3", "c", ClusterRole::ControlPlane, 28),
        ];
        let clusters = [
            cluster("a", ClusterCni::Flannel),
            cluster("b", ClusterCni::Flannel),
            cluster("c", ClusterCni::Flannel),
        ];
        let plans = plan_clusters(&clusters, &members, &[], &management()).unwrap();

        assert_eq!(plans[0].vip, None);
        assert_eq!(plans[0].endpoint, Ipv4Addr::new(172, 31, 1, 21));
        assert_eq!(plans[0].endpoint_url(), "https://172.31.1.21:6443");
        assert!(!plans[0].schedulable_control_planes());
        assert_eq!(plans[0].member_count(), 2);
        assert_eq!(plans[0].role_of("a-w1"), Some(ClusterRole::Worker));
        assert_eq!(plans[0].role_of("b-cp1"), None);

        assert_eq!(plans[1].vip, Some(Ipv4Addr::new(172, 31, 1, 11)));
        assert_eq!(plans[1].endpoint, Ipv4Addr::new(172, 31, 1, 11));
        assert_eq!(plans[1].bootstrap_node().unwrap().name, "b-cp1");
        assert!(plans[1].schedulable_control_planes());
        assert_eq!(plans[2].vip, Some(Ipv4Addr::new(172, 31, 1, 12)));
    }

    #[test]
    fn cilium_clusters_get_load_balancer_blocks_in_manifest_order() {
        let members = [
            member("a-cp1", "a", ClusterRole::ControlPlane, 21),
            member("b-cp1", "b", ClusterRole::ControlPlane, 22),
            member("c-cp1", "c", ClusterRole::ControlPlane, 23),
        ];
        let clusters = [
            cluster("a", ClusterCni::Cilium),
            cluster("b", ClusterCni::Flannel),
            cluster("c", ClusterCni::Cilium),
        ];
        let plans = plan_clusters(&clusters, &members, &[], &management()).unwrap();

        assert_eq!(
            plans[0].load_balancer_addresses,
            Some(AddressRange {
                first: Ipv4Addr::new(172, 31, 1, 13),
                last: Ipv4Addr::new(172, 31, 1, 16),
            })
        );
        assert_eq!(
            plans[0].gateway_address(),
            Some(Ipv4Addr::new(172, 31, 1, 13))
        );
        assert_eq!(
            plans[0].hubble_ui_address(),
            Some(Ipv4Addr::new(172, 31, 1, 14))
        );
        assert_eq!(plans[1].cni, ClusterCni::Flannel);
        assert_eq!(plans[1].load_balancer_addresses, None);
        assert_eq!(plans[1].gateway_address(), None);
        assert_eq!(
            plans[2].load_balancer_addresses,
            Some(AddressRange {
                first: Ipv4Addr::new(172, 31, 1, 17),
                last: Ipv4Addr::new(172, 31, 1, 20),
            })
        );
    }

    #[test]
    fn plans_saved_before_cni_choice_are_flannel_clusters() {
        let saved = r#"
name = "k8s"
endpoint = "172.31.1.21"
workers = []

[[control_planes]]
name = "cp1"
ipv4 = "172.31.1.21"
"#;
        let plan: ClusterPlan = toml::from_str(saved).unwrap();
        assert_eq!(plan.cni, ClusterCni::Flannel);
        assert_eq!(plan.load_balancer_addresses, None);
        assert_eq!(plan.oidc, None);
        assert_eq!(plan.vault, None);
    }

    #[test]
    fn clusters_record_the_keycloak_and_vault_they_use() {
        let members = [
            member("a-cp1", "a", ClusterRole::ControlPlane, 21),
            member("b-cp1", "b", ClusterRole::ControlPlane, 22),
        ];
        let services = [
            ClusterMember {
                name: "kc01".to_owned(),
                ipv4: Ipv4Addr::new(172, 31, 1, 23),
            },
            ClusterMember {
                name: "vault01".to_owned(),
                ipv4: Ipv4Addr::new(172, 31, 1, 24),
            },
        ];
        let clusters = [
            TalosCluster {
                oidc: Some("kc01".to_owned()),
                vault: Some("vault01".to_owned()),
                ..cluster("a", ClusterCni::Flannel)
            },
            cluster("b", ClusterCni::Flannel),
        ];
        let plans = plan_clusters(&clusters, &members, &services, &management()).unwrap();
        assert_eq!(plans[0].oidc.as_ref(), Some(&services[0]));
        assert_eq!(plans[0].vault.as_ref(), Some(&services[1]));
        assert_eq!(plans[1].oidc, None);
        let restored: ClusterPlan = toml::from_str(&toml::to_string(&plans[0]).unwrap()).unwrap();
        assert_eq!(restored, plans[0]);

        let unknown = [TalosCluster {
            vault: Some("vault02".to_owned()),
            ..cluster("a", ClusterCni::Flannel)
        }];
        assert!(plan_clusters(&unknown, &members, &services, &management()).is_err());
    }

    #[test]
    fn cilium_plans_round_trip_through_toml() {
        let members = [member("a-cp1", "a", ClusterRole::ControlPlane, 21)];
        let plans = plan_clusters(
            &[cluster("a", ClusterCni::Cilium)],
            &members,
            &[],
            &management(),
        )
        .unwrap();
        let restored: ClusterPlan = toml::from_str(&toml::to_string(&plans[0]).unwrap()).unwrap();
        assert_eq!(restored, plans[0]);
    }
}
