use std::net::Ipv4Addr;

use anyhow::Result;

use virt::connect::Connect;
use virt::network::Network;

use konst::{
    ARISTA_OUI, ARISTA_VEOS_ZTP_SCRIPT, ARISTA_ZTP_DIR, ARUBA_OUI, ARUBA_ZTP_CONFIG, ARUBA_ZTP_DIR,
    BOOT_SERVER_MAC, BOOT_SERVER_NAME, CISCO_IOSV_OUI, CISCO_IOSV_ZTP_CONFIG, CISCO_IOSXE_OUI,
    CISCO_IOSXE_ZTP_CONFIG, CISCO_IOSXR_OUI, CISCO_IOSXR_ZTP_CONFIG, CISCO_NXOS_OUI,
    CISCO_NXOS_ZTP_CONFIG, CISCO_ZTP_DIR, CUMULUS_OUI, CUMULUS_ZTP_CONFIG, CUMULUS_ZTP_DIR,
    JUNIPER_OUI, JUNIPER_ZTP_DIR, JUNIPER_ZTP_SCRIPT, MTU_JUMBO_NET, SHERPA_DOMAIN_NAME,
};

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
    // Using network namespaces to push config down to dnsmasq.
    // This is required because the DHCP option that can be set
    // via libvirt are limited to only a couple of options
    // and options 67 and 150 are not some of them.
    // https://libvirt.org/formatnetwork.html#network-namespaces
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let network_name = &self.network_name;
        let bridge_name = &self.bridge_name;
        let ipv4_address = &self.ipv4_address;
        let ipv4_netmask = &self.ipv4_netmask;
        let ipv4_default_gateway = &self.ipv4_default_gateway;
        let dhcp_start = &self.dhcp_start;
        let dhcp_end = &self.dhcp_end;
        let ztp_http_port = &self.ztp_http_port;
        let ztp_tftp_port = &self.ztp_tftp_port;
        let ztp_server_ipv4 = &self.ztp_server_ipv4;
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
