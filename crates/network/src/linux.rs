use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::{Handle, LinkBridge, LinkVeth, new_connection};

const MTU_JUMBO_NET: u32 = 9600;

/// Helper to set up netlink connection
async fn setup_netlink() -> Result<Handle> {
    let (connection, handle, _) = new_connection().context("Error creating netlink connection")?;
    tokio::spawn(connection);
    Ok(handle)
}

/// Helper to get a link index
async fn get_link_index(handle: &Handle, name: &str) -> Result<u32> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(msg) = links.try_next().await? {
        Ok(msg.header.index)
    } else {
        anyhow::bail!("link {} not found", name);
    }
}

#[allow(dead_code)]
/// Helper to set a link to up state
async fn enable_link(handle: &Handle, name: &str, index: u32) -> Result<()> {
    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.header.flags = LinkFlags::Up; // IFF_UP = 1
    msg.header.change_mask = LinkFlags::Up; // change the UP flag

    tracing::debug!(link_name = %name, "Enabling link");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error enabling link: {name}"))?;
    Ok(())
}

#[allow(dead_code)]
/// Set MTU on an interface
async fn set_mtu(handle: &Handle, name: &str, mtu: u32) -> Result<()> {
    let index = get_link_index(handle, name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.attributes.push(LinkAttribute::Mtu(mtu));

    tracing::debug!(link_name = %name, mtu = mtu, "Setting MTU on link");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting MTU on: {name}"))?;

    Ok(())
}

#[allow(dead_code)]
/// Set alternate name on an interface
async fn set_alias_name(handle: &Handle, name: &str, alias: &str) -> Result<()> {
    let index = get_link_index(handle, name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.attributes
        .push(LinkAttribute::IfAlias(alias.to_string()));

    tracing::debug!(link_name = %name, alias = %alias, "Setting alias name on link");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting alias: {alias} on: {name}"))?;

    Ok(())
}

/// Set all common link properties at once.
async fn set_link_properties(
    handle: &Handle,
    link_name: &str,
    link_idx: u32,
    link_alias: &str,
    mtu: u32,
) -> Result<()> {
    let mut msg = LinkMessage::default();
    msg.header.index = link_idx;
    msg.header.flags = LinkFlags::Up; // IFF_UP = 1
    msg.header.change_mask = LinkFlags::Up; // change the UP flag
    msg.attributes.extend(vec![
        LinkAttribute::IfAlias(link_alias.to_string()),
        LinkAttribute::Mtu(mtu),
    ]);

    tracing::debug!(link_name = %link_name, alias = %link_alias, mtu = mtu, "Setting link properties");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting properites on: {link_name}"))?;

    Ok(())
}

/// Create a bridge interface
pub async fn create_bridge(name: &str, alias_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // https://interestingtraffic.nl/2017/11/21/an-oddly-specific-post-about-group_fwd_mask/
    //
    // IEEE 802.1D MAC Bridge Filtered MAC Group Addresses: 01-80-C2-00-00-00 to 01-80-C2-00-00-0F;
    // MAC frames that have a destination MAC address within this range are not relayed by MAC bridges
    // conforming to IEEE 802.1D.
    //
    // Allow all protocols except for 00, 01, 02 which are limited in the kernel.
    // 01-80-C2-00-00-00 	Spanning Tree (STP/RSPT/MSTP)
    // 01-80-C2-00-00-01 	Ethernet Flow Control (pause frames)
    // 01-80-C2-00-00-02 	Link Aggregation Control Protocol (LACP)
    let mask: u16 = 0xFFF8;

    tracing::info!(bridge_name = %name, alias = %alias_name, "Creating bridge");
    handle
        .link()
        .add(LinkBridge::new(name).group_fwd_mask(mask).build())
        .execute()
        .await
        .context(format!("Error creating bridge: {name}"))?;

    let idx = get_link_index(&handle, name).await?;

    set_link_properties(&handle, name, idx, alias_name, MTU_JUMBO_NET).await?;

    Ok(())
}

pub async fn create_veth_pair(
    src_name: &str,
    dst_name: &str,
    src_alias_name: &str,
    dst_alias_name: &str,
) -> Result<()> {
    let handle = setup_netlink().await?;

    // Create veth pair
    tracing::info!(src_name = %src_name, dst_name = %dst_name, "Creating veth pair");
    handle
        .link()
        .add(LinkVeth::new(src_name, dst_name).build())
        .execute()
        .await
        .context(format!(
            "Error creating veth pair: {src_name} <--> {dst_name}"
        ))?;

    let src_idx = get_link_index(&handle, src_name).await?;
    set_link_properties(&handle, src_name, src_idx, src_alias_name, MTU_JUMBO_NET).await?;

    let dst_idx = get_link_index(&handle, dst_name).await?;
    set_link_properties(&handle, dst_name, dst_idx, dst_alias_name, MTU_JUMBO_NET).await?;

    Ok(())
}

/// Enslave a veth interface to a bridge
pub async fn enslave_to_bridge(int_name: &str, bridge_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Get the veth interface
    let mut veth_links = handle
        .link()
        .get()
        .match_name(int_name.to_string())
        .execute();
    let veth_link = if let Some(link) = veth_links.try_next().await? {
        link
    } else {
        return Err(anyhow!("Interface {} not found", int_name));
    };

    // Get the bridge interface index
    let bridge_idx = get_link_index(&handle, bridge_name).await?;

    // Create a minimal LinkMessage for the change
    let mut msg = LinkMessage::default();
    msg.header.index = veth_link.header.index;
    msg.attributes.push(LinkAttribute::Controller(bridge_idx));

    tracing::debug!(interface_name = %int_name, bridge_name = %bridge_name, "Enslaving interface to bridge");
    handle.link().change(msg).execute().await.context(format!(
        "Error setting interface as bridge slave.\n int: {int_name} - bridge: {bridge_name}"
    ))?;

    Ok(())
}

/// Delete and interface
pub async fn delete_interface(name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Get the interface index
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(link) = links.try_next().await? {
        tracing::info!(interface_name = %name, "Deleting interface");
        handle.link().del(link.header.index).execute().await?;
        Ok(())
    } else {
        Err(anyhow!("Interface {} not found", name))
    }
}

/// Find interfaces by fuzzy match
pub async fn find_interfaces_fuzzy(fuzzy_name: &str) -> Result<Vec<String>> {
    let handle = setup_netlink().await?;

    // Get all links
    let mut links = handle.link().get().execute();

    let mut intefaces = Vec::new();

    while let Some(link) = links.try_next().await? {
        // Extract the interface name from attributes
        for attr in link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                if name.contains(fuzzy_name) {
                    intefaces.push(name);
                }
                break;
            }
        }
    }

    Ok(intefaces)
}
