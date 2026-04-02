use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use askama::Template;
use tracing::instrument;
use virt::connect::Connect;
use virt::network::Network;

use shared::konst::{MTU_JUMBO_NET, SHERPA_DOMAIN_NAME};

/// Ensure an existing libvirt network is active. Returns true if the network
/// exists (and is now active), false if it doesn't exist at all.
fn ensure_network_active(qemu_conn: &Connect, name: &str) -> Result<bool> {
    let network = match Network::lookup_by_name(qemu_conn, name) {
        Ok(n) => n,
        Err(_) => return Ok(false),
    };

    let active = network
        .is_active()
        .with_context(|| format!("Failed to check if network '{name}' is active"))?;

    if !active {
        tracing::info!(network_name = %name, "Network exists but is inactive, starting it");
        network
            .create()
            .with_context(|| format!("Failed to start existing network '{name}'"))?;
    }

    Ok(true)
}

/// Define, start, and autostart a libvirt network from XML.
fn create_network(qemu_conn: &Connect, xml: &str, kind: &str) -> Result<()> {
    let network = Network::define_xml(qemu_conn, xml)
        .with_context(|| format!("Failed to define {kind} network"))?;
    network
        .create()
        .with_context(|| format!("Failed to start {kind} network"))?;
    network
        .set_autostart(true)
        .with_context(|| format!("Failed to set {kind} network autostart"))?;
    Ok(())
}

#[derive(Template)]
#[template(path = "network/bridge.jinja", ext = "xml")]
pub struct BridgeNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl BridgeNetwork {
    #[instrument(level = "debug", skip(self, qemu_conn))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if ensure_network_active(qemu_conn, &self.network_name)? {
            tracing::debug!(network_name = %self.network_name, "Bridge network already exists");
            return Ok(());
        }

        let xml = self
            .render()
            .context("Failed to render bridge network XML")?;
        create_network(qemu_conn, &xml, "bridge")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Bridge network created and started");
        Ok(())
    }
}

#[derive(Template)]
#[template(path = "network/isolated.jinja", ext = "xml")]
pub struct IsolatedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl IsolatedNetwork {
    /// Create an isolated bridge for forwarding disabled and ports
    /// isolated from one another.
    #[instrument(level = "debug", skip(self, qemu_conn))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if ensure_network_active(qemu_conn, &self.network_name)? {
            tracing::debug!(network_name = %self.network_name, "Isolated network already exists");
            return Ok(());
        }

        let xml = self
            .render()
            .context("Failed to render isolated network XML")?;
        create_network(qemu_conn, &xml, "isolated")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Isolated network created and started");
        Ok(())
    }
}

#[derive(Template)]
#[template(path = "network/reserved.jinja", ext = "xml")]
pub struct ReservedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl ReservedNetwork {
    /// Create a reserved bridge for control traffic in a VM.
    #[instrument(level = "debug", skip(self, qemu_conn))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if ensure_network_active(qemu_conn, &self.network_name)? {
            tracing::debug!(network_name = %self.network_name, "Reserved network already exists");
            return Ok(());
        }

        let xml = self
            .render()
            .context("Failed to render reserved network XML")?;
        create_network(qemu_conn, &xml, "reserved")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Reserved network created and started");
        Ok(())
    }
}

#[derive(Template)]
#[template(path = "network/nat.jinja", ext = "xml")]
pub struct NatNetwork {
    pub network_name: String,
    pub bridge_name: String,
    pub ipv4_address: Ipv4Addr,
    pub ipv4_netmask: Ipv4Addr,
    pub ipv6_address: Option<Ipv6Addr>,
    pub ipv6_prefix_length: Option<u8>,
}

impl NatNetwork {
    #[instrument(level = "debug", skip(self, qemu_conn))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if ensure_network_active(qemu_conn, &self.network_name)? {
            tracing::debug!(network_name = %self.network_name, "NAT network already exists");
            return Ok(());
        }

        let xml = self.render().context("Failed to render NAT network XML")?;
        create_network(qemu_conn, &xml, "NAT")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, ipv4_address = %self.ipv4_address, "NAT network created and started");
        Ok(())
    }
}
