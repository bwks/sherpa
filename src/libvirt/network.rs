use anyhow::Result;

use virt::connect::Connect;
use virt::network::Network;

use crate::core::konst::{ARISTA_OUI, CISCO_OUI, JUNIPER_OUI, MTU_JUMBO_NET};

/// Create an isolated bridge for forwarding disabled and ports
/// isolated from one another.
pub fn create_isolated_network(conn: &Connect, name: &str, bridge_name: &str) -> Result<()> {
    let network_xml = format!(
        r#"
        <network>
          <name>{name}</name>
          <mtu size="{MTU_JUMBO_NET}"/>
          <bridge name='{bridge_name}' stp='on' delay='0'/>
          <forward mode='none'/>
          <port isolated='yes'/>
        </network>
        "#,
    );

    let network = Network::define_xml(conn, &network_xml)?;
    network.create()?;
    network.set_autostart(true)?;

    println!("Network created and started: {name}");

    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Create a virtual network with dhcp enabled.
pub fn create_network(
    conn: &Connect,
    name: &str,
    bridge_name: &str,
    ip_address: &str,
    netmask: &str,
    dhcp_start: &str,
    dhcp_end: &str,
    boot_server: &str,
) -> Result<()> {
    // Using network namespaces to push config down to dnsmasq.
    // This is required because the DHCP option that can be set
    // via libvirt are limited to only a couple of options
    // and options 67 and 150 are not some of them.
    // https://libvirt.org/formatnetwork.html#network-namespaces
    let network_xml = format!(
        r#"
        <network connections='1' xmlns:dnsmasq='http://libvirt.org/schemas/network/dnsmasq/1.0'>
          <dnsmasq:options>

            <dnsmasq:option value="dhcp-option-force=tag:arista,67,http://{boot_server}/arista/bootstrap.py"/>
            <dnsmasq:option value="dhcp-option-force=tag:cisco,67,http://{boot_server}/cisco/bootstrap.py"/>
            <dnsmasq:option value="dhcp-option-force=tag:juniper,67,http://{boot_server}/juniper/bootstrap.py"/>

            <dnsmasq:option value="dhcp-mac=set:arista,{ARISTA_OUI}:*:*:*"/>
            <dnsmasq:option value="dhcp-mac=set:cisco,{CISCO_OUI}:*:*:*"/>
            <dnsmasq:option value="dhcp-mac=set:juniper,{JUNIPER_OUI}:*:*:*"/>

            <dnsmasq:option value="dhcp-option=150,{boot_server}"/>
          </dnsmasq:options>

          <name>{name}</name>
          
          <mtu size="{MTU_JUMBO_NET}"/>
          
          <forward mode='nat'>
            <nat>
              <port start='1024' end='65535'/>
            </nat>
          </forward>

          <bridge name='{bridge_name}' stp='on' delay='0'/>
          
          <domain name='sherpa.lab.local' localOnly='yes'/>
          
          <ip address='{ip_address}' netmask='{netmask}'>
            <dhcp>
              <range start='{dhcp_start}' end='{dhcp_end}'/>
            </dhcp>
          </ip>
        
        </network>
        "#
    );

    let network = Network::define_xml(conn, &network_xml)?;
    network.create()?;
    network.set_autostart(true)?;

    println!("Network created and started: {name}");

    Ok(())
}
