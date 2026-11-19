use std::fmt;
use std::net::Ipv4Addr;

use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};

use crate::konst::KUBERNETES_API_PORT;

/// Role of a node in a Kubernetes cluster declared in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClusterRole {
    /// Runs etcd and the Kubernetes control plane.
    ControlPlane,
    /// Runs workloads only.
    Worker,
}

/// The CNI of a Kubernetes cluster declared in the manifest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClusterCni {
    /// Flannel, deployed by Talos.
    #[default]
    Flannel,
    /// Cilium, replacing kube-proxy, with Gateway API, load balancer addresses announced on
    /// the management network, and Hubble.
    Cilium,
}

impl ClusterCni {
    #[must_use]
    pub fn is_flannel(&self) -> bool {
        *self == ClusterCni::Flannel
    }
}

/// A node a cluster is made of or uses, and its management address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterMember {
    pub name: String,
    pub ipv4: Ipv4Addr,
}

/// Management addresses from `first` to `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressRange {
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
}

/// Addresses and members of one Talos cluster: the manifest's cluster, with the addresses the
/// server chose for it at `up`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterPlan {
    pub name: String,
    /// Address the Kubernetes API is reached on: the shared address with several control
    /// planes, otherwise the only control plane's address.
    pub endpoint: Ipv4Addr,
    /// Shared address the control planes elect an owner for.
    pub vip: Option<Ipv4Addr>,
    /// The cluster's CNI.
    pub cni: ClusterCni,
    /// Addresses a Cilium cluster hands out to load balancers: its Gateway first, then Hubble
    /// UI.
    pub load_balancer_addresses: Option<AddressRange>,
    /// Control planes in manifest order. The first one is bootstrapped.
    pub control_planes: Vec<ClusterMember>,
    pub workers: Vec<ClusterMember>,
    /// The Keycloak node whose realm users log in to the cluster.
    pub oidc: Option<ClusterMember>,
    /// The Vault node the cluster's workloads read secrets from.
    pub vault: Option<ClusterMember>,
}

impl ClusterPlan {
    #[must_use]
    pub fn endpoint_url(&self) -> String {
        format!("https://{}:{KUBERNETES_API_PORT}", self.endpoint)
    }

    /// With no workers, workloads must be allowed on the control planes.
    #[must_use]
    pub fn schedulable_control_planes(&self) -> bool {
        self.workers.is_empty()
    }

    /// The one control plane that receives the bootstrap request.
    #[must_use]
    pub fn bootstrap_node(&self) -> Option<&ClusterMember> {
        self.control_planes.first()
    }

    #[must_use]
    pub fn member_count(&self) -> usize {
        self.control_planes.len() + self.workers.len()
    }

    /// Address of a Cilium cluster's Gateway: the first load balancer address.
    #[must_use]
    pub fn gateway_address(&self) -> Option<Ipv4Addr> {
        self.load_balancer_addresses.map(|range| range.first)
    }

    /// Address of a Cilium cluster's Hubble UI: the load balancer address after the Gateway's.
    #[must_use]
    pub fn hubble_ui_address(&self) -> Option<Ipv4Addr> {
        self.gateway_address()
            .map(|gateway| Ipv4Addr::from(u32::from(gateway) + 1))
    }

    /// The member `node_name`, with its role, if it is one of the cluster's.
    #[must_use]
    pub fn member(&self, node_name: &str) -> Option<(ClusterRole, &ClusterMember)> {
        let control_planes = self
            .control_planes
            .iter()
            .map(|member| (ClusterRole::ControlPlane, member));
        let workers = self
            .workers
            .iter()
            .map(|member| (ClusterRole::Worker, member));
        control_planes
            .chain(workers)
            .find(|(_, member)| member.name == node_name)
    }

    #[must_use]
    pub fn role_of(&self, node_name: &str) -> Option<ClusterRole> {
        self.member(node_name).map(|(role, _)| role)
    }
}

impl fmt::Display for ClusterRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterRole::ControlPlane => write!(f, "control_plane"),
            ClusterRole::Worker => write!(f, "worker"),
        }
    }
}

impl fmt::Display for ClusterCni {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterCni::Flannel => write!(f, "flannel"),
            ClusterCni::Cilium => write!(f, "cilium"),
        }
    }
}
