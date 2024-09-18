use anyhow::Result;

use virt::connect::Connect;
use virt::network::Network;

/// Create an isolated bridge.
pub fn create_isolated_network(conn: &Connect, name: &str, bridge_name: &str) -> Result<()> {
    let network_xml = format!(
        r#"
        <network>
          <name>{name}</name>
          <bridge name='{bridge_name}' stp='on' delay='0'/>
        </network>
        "#,
    );

    let network = Network::define_xml(conn, &network_xml)?;
    network.create()?;
    network.set_autostart(true)?;

    println!("Network created and started: {name}");

    Ok(())
}

/// Create a virtual network with dhcp enabled.
pub fn create_network(
    conn: &Connect,
    name: &str,
    bridge_name: &str,
    ip_address: &str,
    netmask: &str,
    dhcp_start: &str,
    dhcp_end: &str,
) -> Result<()> {
    let network_xml = format!(
        r#"
        <network connections='1'>
          <name>{name}</name>
          <forward mode='nat'>
            <nat>
              <port start='1024' end='65535'/>
            </nat>
          </forward>
          <bridge name='{bridge_name}' stp='on' delay='0'/>
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
