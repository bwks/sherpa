use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use shared::data::{ClusterRole, NetworkV4};
use shared::konst::TALOS_VIP_HOST_OFFSETS;
use shared::util::get_ipv4_addr;

/// Kubernetes API port.
pub const KUBERNETES_API_PORT: u16 = 6443;

/// A cluster node and its management address.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Addresses and members of one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterPlan {
    pub name: String,
    /// Address the Kubernetes API is reached on: the shared address with several control
    /// planes, otherwise the only control plane's address.
    pub endpoint: Ipv4Addr,
    /// Shared address the control planes elect an owner for.
    pub vip: Option<Ipv4Addr>,
    /// Control planes in manifest order. The first one is bootstrapped.
    pub control_planes: Vec<ClusterMember>,
    pub workers: Vec<ClusterMember>,
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

/// Plan every cluster: assign shared addresses to clusters with several control planes, in
/// manifest order, from the reserved offsets in the management subnet. Clusters must already be validated.
pub fn plan_clusters(
    cluster_names: &[&str],
    members: &[MemberInput],
    management: &NetworkV4,
) -> Result<Vec<ClusterPlan>> {
    let mut vip_offsets = TALOS_VIP_HOST_OFFSETS;
    let mut plans = Vec::with_capacity(cluster_names.len());
    for name in cluster_names {
        let mut control_planes = vec![];
        let mut workers = vec![];
        for member in members.iter().filter(|member| member.cluster == *name) {
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
        plans.push(ClusterPlan {
            name: (*name).to_owned(),
            endpoint: vip.unwrap_or(first_control_plane),
            vip,
            control_planes,
            workers,
        });
    }
    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let management = NetworkV4 {
            prefix: "172.31.1.0/24".parse().unwrap(),
            first: "172.31.1.1".parse().unwrap(),
            last: "172.31.1.255".parse().unwrap(),
            boot_server: "172.31.1.2".parse().unwrap(),
            network: "172.31.1.0".parse().unwrap(),
            subnet_mask: "255.255.255.0".parse().unwrap(),
            hostmask: "0.0.0.255".parse().unwrap(),
            prefix_length: 24,
        };
        let members = [
            member("a-cp1", "a", ClusterRole::ControlPlane, 10),
            member("a-w1", "a", ClusterRole::Worker, 11),
            member("b-cp1", "b", ClusterRole::ControlPlane, 12),
            member("b-cp2", "b", ClusterRole::ControlPlane, 13),
            member("c-cp1", "c", ClusterRole::ControlPlane, 14),
            member("c-cp2", "c", ClusterRole::ControlPlane, 15),
        ];
        let plans = plan_clusters(&["a", "b", "c"], &members, &management).unwrap();

        assert_eq!(plans[0].vip, None);
        assert_eq!(plans[0].endpoint, Ipv4Addr::new(172, 31, 1, 10));
        assert_eq!(plans[0].endpoint_url(), "https://172.31.1.10:6443");
        assert!(!plans[0].schedulable_control_planes());
        assert_eq!(plans[0].member_count(), 2);
        assert_eq!(plans[0].role_of("a-w1"), Some(ClusterRole::Worker));
        assert_eq!(plans[0].role_of("b-cp1"), None);

        assert_eq!(plans[1].vip, Some(Ipv4Addr::new(172, 31, 1, 3)));
        assert_eq!(plans[1].endpoint, Ipv4Addr::new(172, 31, 1, 3));
        assert_eq!(plans[1].bootstrap_node().unwrap().name, "b-cp1");
        assert!(plans[1].schedulable_control_planes());
        assert_eq!(plans[2].vip, Some(Ipv4Addr::new(172, 31, 1, 4)));
    }
}
