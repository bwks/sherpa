//! What re-attaching links and setting their carrier does to the host, behind a trait so the
//! transitions can be tested without a kernel, Docker or libvirt.

use std::sync::Arc;

use anyhow::{Context, Result};
use bollard::Docker;
use libvirt::Qemu;
use tracing::instrument;

use super::veths::ContainerVeth;
use crate::services::node_ops;

use shared::data::LinkImpairment;
use shared::util;

/// One end of a link, as its carrier is read and set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndDevice {
    /// A container's host veth: setting it down takes the carrier from its peer in the
    /// container.
    Veth(String),
    /// The host tap of an interface of libvirt domain `domain`. Its carrier is the interface's
    /// link state in the live domain: a tap set down is not seen by the guest.
    Tap { domain: String, tap: String },
}

/// The host operations that re-attach a link: look up an end and whether it has its redirect,
/// point its redirect at its peer, set its impairment, make a container's veths, and read and
/// set an end's carrier.
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

    /// Whether end `end` has carrier, or `None` when it does not exist: a veth that is not
    /// there, a domain that is not running.
    async fn carrier(&self, end: &EndDevice) -> Result<Option<bool>>;

    /// Give end `end` carrier, or take it away.
    async fn set_carrier(&self, end: &EndDevice, up: bool) -> Result<()>;
}

/// The real host: the kernel through `network`, Docker, and libvirt.
pub(crate) struct KernelHost<'a> {
    docker: &'a Docker,
    qemu: &'a Arc<Qemu>,
}

impl<'a> KernelHost<'a> {
    pub(crate) fn new(docker: &'a Docker, qemu: &'a Arc<Qemu>) -> Self {
        Self { docker, qemu }
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
    async fn carrier(&self, end: &EndDevice) -> Result<Option<bool>> {
        match end {
            EndDevice::Veth(iface) => network::link_is_up(iface).await,
            EndDevice::Tap { domain, tap } => {
                let qemu = Arc::clone(self.qemu);
                let (domain, tap) = (domain.clone(), tap.clone());
                tokio::task::spawn_blocking(move || {
                    let conn = qemu.connect()?;
                    libvirt::interface_link_up(&conn, &domain, &tap)
                })
                .await
                .context("The libvirt link state read panicked")?
            }
        }
    }

    #[instrument(skip(self), level = "debug")]
    async fn set_carrier(&self, end: &EndDevice, up: bool) -> Result<()> {
        match end {
            EndDevice::Veth(iface) => network::set_link_state(iface, up).await,
            EndDevice::Tap { domain, tap } => {
                let qemu = Arc::clone(self.qemu);
                let (domain, tap) = (domain.clone(), tap.clone());
                tokio::task::spawn_blocking(move || {
                    let conn = qemu.connect()?;
                    libvirt::set_interface_link(&conn, &domain, &tap, up)
                })
                .await
                .context("The libvirt link state change panicked")?
            }
        }
    }
}
