//! What re-attaching links does to the host, behind a trait so the transitions can be tested
//! without a kernel or Docker.

use anyhow::{Context, Result};
use bollard::Docker;
use tracing::instrument;

use super::veths::ContainerVeth;
use crate::services::node_ops;

use shared::data::LinkImpairment;
use shared::konst::CONTAINER_MGMT_INTERFACE;
use shared::util;

/// The host operations that re-attach a link: look up an end and whether it has its redirect,
/// point its redirect at its peer, set its impairment, and make a container's veths. Also those
/// that keep transmit checksum offload off on a container's management veth.
pub(crate) trait LinkHost {
    /// The index of interface `iface`, or `None` when it does not exist.
    async fn ifindex(&self, iface: &str) -> Result<Option<u32>>;

    /// Whether the interface with index `ifindex` has the P2p redirect on its ingress. An end
    /// made again never has it.
    async fn has_redirect(&self, ifindex: u32) -> Result<bool>;

    /// Redirect what arrives on `iface` to the interface with index `peer_ifindex`, replacing
    /// any redirect it has.
    fn attach_redirect(&self, iface: &str, peer_ifindex: u32) -> Result<()>;

    /// Make the egress impairment of the interface with index `ifindex` be `impairment`: set,
    /// or removed when it is clear.
    async fn sync_netem(&self, ifindex: u32, impairment: &LinkImpairment) -> Result<()>;

    /// The PID of container `container`, whose network namespace its veths go in.
    async fn container_pid(&self, container: &str) -> Result<u32>;

    /// Make `veth` in the network namespace of process `pid`, and join its host end to its
    /// bridge, if it has one. For a model whose container needs finished checksums, transmit
    /// checksum offload goes off on the host end.
    async fn create_veth(&self, veth: &ContainerVeth, pid: u32) -> Result<()>;

    /// The index of the host end of container `container`'s management veth.
    async fn management_veth(&self, container: &str) -> Result<u32>;

    /// Whether transmit checksum offload is on for the interface with index `ifindex`.
    fn tx_checksum_offload_on(&self, ifindex: u32) -> Result<bool>;

    /// Turn transmit checksum offload off on the interface with index `ifindex`.
    fn disable_tx_checksum_offload(&self, ifindex: u32) -> Result<()>;
}

/// The real host: the kernel through `network`, and Docker.
pub(crate) struct KernelHost<'a> {
    docker: &'a Docker,
}

impl<'a> KernelHost<'a> {
    pub(crate) fn new(docker: &'a Docker) -> Self {
        Self { docker }
    }
}

impl LinkHost for KernelHost<'_> {
    async fn ifindex(&self, iface: &str) -> Result<Option<u32>> {
        network::find_ifindex(iface).await
    }

    async fn has_redirect(&self, ifindex: u32) -> Result<bool> {
        network::has_p2p_redirect(ifindex)
            .await
            .with_context(|| format!("failed to read the redirect of ifindex {ifindex}"))
    }

    fn attach_redirect(&self, iface: &str, peer_ifindex: u32) -> Result<()> {
        network::attach_p2p_redirect(iface, peer_ifindex)
            .with_context(|| format!("failed to attach the eBPF redirect on {iface}"))
    }

    async fn sync_netem(&self, ifindex: u32, impairment: &LinkImpairment) -> Result<()> {
        network::sync_netem(ifindex, impairment)
            .await
            .with_context(|| format!("failed to set the impairment on ifindex {ifindex}"))
    }

    async fn container_pid(&self, container: &str) -> Result<u32> {
        container::get_container_pid(self.docker, container).await
    }

    #[instrument(skip(self, veth), fields(host_veth = %veth.host_veth, pid), level = "debug")]
    async fn create_veth(&self, veth: &ContainerVeth, pid: u32) -> Result<()> {
        let target_name = util::container_interface_name(&veth.node_model, veth.interface_idx)?;
        let state = node_ops::netns_veth_state(veth.node_model, veth.admin_down);
        network::create_veth_in_netns(&veth.host_veth, &veth.host_alias, &target_name, pid, state)
            .await
            .with_context(|| {
                format!("failed to create interface {target_name} in the container")
            })?;
        if veth.node_model.needs_host_checksums() {
            let ifindex = network::get_ifindex(&veth.host_veth).await?;
            network::disable_tx_checksum_offload(ifindex).with_context(|| {
                format!(
                    "failed to turn transmit checksum offload off on {}",
                    veth.host_veth
                )
            })?;
        }
        if let Some(bridge_name) = &veth.bridge {
            network::enslave_to_bridge(&veth.host_veth, bridge_name).await?;
        }
        tracing::info!(
            host_veth = %veth.host_veth,
            target = %target_name,
            admin_down = veth.admin_down,
            pid,
            "Created container veth"
        );
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    async fn management_veth(&self, container: &str) -> Result<u32> {
        let pid = container::get_container_pid(self.docker, container).await?;
        network::container_veth_peer_ifindex(pid, CONTAINER_MGMT_INTERFACE).await
    }

    fn tx_checksum_offload_on(&self, ifindex: u32) -> Result<bool> {
        network::tx_checksum_offload_on(ifindex)
    }

    fn disable_tx_checksum_offload(&self, ifindex: u32) -> Result<()> {
        network::disable_tx_checksum_offload(ifindex)
    }
}
