use std::net::Ipv4Addr;

use anyhow::{Context, Result};

use virt::connect::Connect;
use virt::network::Network;

use shared::konst::{MTU_JUMBO_NET, SHERPA_DOMAIN_NAME};

/// Check if a libvirt network already exists (active or defined).
fn network_exists(qemu_conn: &Connect, name: &str) -> bool {
    Network::lookup_by_name(qemu_conn, name).is_ok()
}

pub struct BridgeNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl BridgeNetwork {
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if network_exists(qemu_conn, &self.network_name) {
            tracing::debug!(network_name = %self.network_name, "Bridge network already exists");
            return Ok(());
        }

        let network_name = &self.network_name;
        let bridge_name = &self.bridge_name;
        let network_xml = format!(
            r#"
            <network>
              <name>{network_name}</name>
              <forward mode="bridge"/>
              <bridge name="{bridge_name}"/>
            </network>
            "#
        );
        let network = Network::define_xml(qemu_conn, &network_xml)
            .context("Failed to define bridge network")?;
        network.create().context("Failed to start bridge network")?;
        network
            .set_autostart(true)
            .context("Failed to set bridge network autostart")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Bridge network created and started");

        Ok(())
    }
}

pub struct IsolatedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl IsolatedNetwork {
    /// Create an isolated bridge for forwarding disabled and ports
    /// isolated from one another.
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if network_exists(qemu_conn, &self.network_name) {
            tracing::debug!(network_name = %self.network_name, "Isolated network already exists");
            return Ok(());
        }

        let network_name = &self.network_name;
        let bridge_name = &self.bridge_name;
        let network_xml = format!(
            r#"
      <network>
        <name>{network_name}</name>
        <mtu size="{MTU_JUMBO_NET}"/>
        <bridge name='{bridge_name}' stp='on' delay='0'/>
        <forward mode='none'/>
        <port isolated='yes'/>
      </network>
      "#
        );

        let network = Network::define_xml(qemu_conn, &network_xml)
            .context("Failed to define isolated network")?;
        network
            .create()
            .context("Failed to start isolated network")?;
        network
            .set_autostart(true)
            .context("Failed to set isolated network autostart")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Isolated network created and started");

        Ok(())
    }
}

pub struct ReservedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl ReservedNetwork {
    /// Create an reserved bridge for control traffic in a VM.
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if network_exists(qemu_conn, &self.network_name) {
            tracing::debug!(network_name = %self.network_name, "Reserved network already exists");
            return Ok(());
        }

        let network_name = &self.network_name;
        let bridge_name = &self.bridge_name;
        let network_xml = format!(
            r#"
      <network>
        <name>{network_name}</name>
        <mtu size="{MTU_JUMBO_NET}"/>
        <bridge name='{bridge_name}' stp='on' delay='0'/>
        <forward mode='none'/>
        <port isolated='no'/>
      </network>
      "#
        );

        let network = Network::define_xml(qemu_conn, &network_xml)
            .context("Failed to define reserved network")?;
        network
            .create()
            .context("Failed to start reserved network")?;
        network
            .set_autostart(true)
            .context("Failed to set reserved network autostart")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, "Reserved network created and started");

        Ok(())
    }
}

pub struct NatNetwork {
    pub network_name: String,
    pub bridge_name: String,
    pub ipv4_address: Ipv4Addr,
    pub ipv4_netmask: Ipv4Addr,
}
impl NatNetwork {
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        if network_exists(qemu_conn, &self.network_name) {
            tracing::debug!(network_name = %self.network_name, "NAT network already exists");
            return Ok(());
        }

        let network_name = &self.network_name;
        let bridge_name = &self.bridge_name;
        let ipv4_address = &self.ipv4_address;
        let ipv4_netmask = &self.ipv4_netmask;
        let network_xml = format!(
            r#"
        <network connections='1' xmlns:dnsmasq='http://libvirt.org/schemas/network/dnsmasq/1.0'>
          <name>{network_name}</name>
          <mtu size="{MTU_JUMBO_NET}"/>
          <forward mode='nat'>
            <nat>
              <port start='1024' end='65535'/>
            </nat>
          </forward>
          <bridge name='{bridge_name}' stp='on' delay='0'/>
          <domain name='{SHERPA_DOMAIN_NAME}' localOnly='yes'/>
          <dns enable='yes'/>
          <ip address='{ipv4_address}' netmask='{ipv4_netmask}'>
          </ip>

        </network>
        "#
        );

        let network =
            Network::define_xml(qemu_conn, &network_xml).context("Failed to define NAT network")?;
        network.create().context("Failed to start NAT network")?;
        network
            .set_autostart(true)
            .context("Failed to set NAT network autostart")?;

        tracing::info!(network_name = %self.network_name, bridge_name = %self.bridge_name, ipv4_address = %self.ipv4_address, "NAT network created and started");

        Ok(())
    }
}
