use std::net::Ipv4Addr;

use anyhow::Result;

use virt::connect::Connect;
use virt::network::Network;

use konst::{MTU_JUMBO_NET, SHERPA_DOMAIN_NAME};

pub struct IsolatedNetwork {
    pub network_name: String,
    pub bridge_name: String,
}

impl IsolatedNetwork {
    /// Create an isolated bridge for forwarding disabled and ports
    /// isolated from one another.
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
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

        let network = Network::define_xml(qemu_conn, &network_xml)?;
        network.create()?;
        network.set_autostart(true)?;

        println!("Network created and started: {}", &self.network_name);

        Ok(())
    }
}

// TODO
// pub struct BridgeNetwork {}

pub struct NatNetwork {
    pub network_name: String,
    pub bridge_name: String,
    pub ipv4_address: Ipv4Addr,
    pub ipv4_netmask: Ipv4Addr,
    pub ipv4_default_gateway: Ipv4Addr,
    pub dhcp_start: Ipv4Addr,
    pub dhcp_end: Ipv4Addr,
    pub ztp_http_port: u16,
    pub ztp_tftp_port: u16,
    pub ztp_server_ipv4: Ipv4Addr,
}
impl NatNetwork {
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
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

        let network = Network::define_xml(qemu_conn, &network_xml)?;
        network.create()?;
        network.set_autostart(true)?;

        println!("Network created and started: {}", &self.network_name);

        Ok(())
    }
}
