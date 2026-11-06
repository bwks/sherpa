use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use askama::Template;
use tracing::instrument;
use virt::connect::Connect;
use virt::error::ErrorNumber;
use virt::network::Network;

use shared::konst::{MTU_JUMBO_NET, SHERPA_DOMAIN_NAME};

/// Ensure an existing libvirt network is active. Returns true if the network
/// exists (and is now active), false if it doesn't exist at all.
fn ensure_network_active(qemu_conn: &Connect, name: &str) -> Result<bool> {
    let network = match Network::lookup_by_name(qemu_conn, name) {
        Ok(network) => network,
        Err(error) if error.code() == ErrorNumber::NoNetwork => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to look up network '{name}'"));
        }
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

/// Make sure the network `name` exists and is active: start it if it is inactive, or define,
/// start and autostart it from `xml`. `kind` names the network in messages.
fn ensure_network(
    qemu_conn: &Connect,
    name: &str,
    bridge_name: &str,
    kind: &str,
    xml: &str,
) -> Result<()> {
    if ensure_network_active(qemu_conn, name)? {
        tracing::debug!(network_name = %name, kind, "Network already exists");
        return Ok(());
    }

    let network = Network::define_xml(qemu_conn, xml)
        .with_context(|| format!("Failed to define {kind} network"))?;
    network
        .create()
        .with_context(|| format!("Failed to start {kind} network"))?;
    network
        .set_autostart(true)
        .with_context(|| format!("Failed to set {kind} network autostart"))?;

    tracing::info!(network_name = %name, %bridge_name, kind, "Network created and started");
    Ok(())
}

/// A network over an existing host bridge.
#[derive(Debug, Template)]
#[template(path = "network/bridge.jinja", ext = "xml")]
pub struct BridgeNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl BridgeNetwork {
    /// Make sure the bridge network exists and is active.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be rendered, or libvirt cannot look up, define, start or
    /// autostart the network.
    #[instrument(level = "debug", skip(self, qemu_conn), fields(network_name = %self.network_name))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let xml = self
            .render()
            .context("Failed to render bridge network XML")?;
        ensure_network(
            qemu_conn,
            &self.network_name,
            &self.bridge_name,
            "bridge",
            &xml,
        )
    }
}

/// A bridge with forwarding disabled and its ports isolated from one another.
#[derive(Debug, Template)]
#[template(path = "network/isolated.jinja", ext = "xml")]
pub struct IsolatedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl IsolatedNetwork {
    /// Make sure the isolated network exists and is active.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be rendered, or libvirt cannot look up, define, start or
    /// autostart the network.
    #[instrument(level = "debug", skip(self, qemu_conn), fields(network_name = %self.network_name))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let xml = self
            .render()
            .context("Failed to render isolated network XML")?;
        ensure_network(
            qemu_conn,
            &self.network_name,
            &self.bridge_name,
            "isolated",
            &xml,
        )
    }
}

/// A bridge for control traffic inside a VM.
#[derive(Debug, Template)]
#[template(path = "network/reserved.jinja", ext = "xml")]
pub struct ReservedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl ReservedNetwork {
    /// Make sure the reserved network exists and is active.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be rendered, or libvirt cannot look up, define, start or
    /// autostart the network.
    #[instrument(level = "debug", skip(self, qemu_conn), fields(network_name = %self.network_name))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let xml = self
            .render()
            .context("Failed to render reserved network XML")?;
        ensure_network(
            qemu_conn,
            &self.network_name,
            &self.bridge_name,
            "reserved",
            &xml,
        )
    }
}

/// A NAT network with DNS, the management network.
#[derive(Debug, Template)]
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
    /// Make sure the NAT network exists and is active.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be rendered, or libvirt cannot look up, define, start or
    /// autostart the network.
    #[instrument(level = "debug", skip(self, qemu_conn), fields(network_name = %self.network_name))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let xml = self.render().context("Failed to render NAT network XML")?;
        ensure_network(
            qemu_conn,
            &self.network_name,
            &self.bridge_name,
            "NAT",
            &xml,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nat_network_bridge_has_no_stp() {
        // With STP on, the kernel holds each new port for twice its 2 second minimum forward
        // delay, so a container's first network traffic is lost.
        let xml = NatNetwork {
            network_name: "sherpa-management-fe4edfdc".to_string(),
            bridge_name: "brm-fe4edfdc".to_string(),
            ipv4_address: Ipv4Addr::new(172, 31, 3, 1),
            ipv4_netmask: Ipv4Addr::new(255, 255, 255, 0),
            ipv6_address: None,
            ipv6_prefix_length: None,
        }
        .render()
        .unwrap();
        assert!(
            xml.contains("<bridge name='brm-fe4edfdc' stp='off'/>"),
            "{xml}"
        );
    }
}
