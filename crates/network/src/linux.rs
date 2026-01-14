use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::{Handle, LinkBridge, LinkVeth, new_connection};

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

/// Helper to set a link to up state
async fn set_link_up(handle: &Handle, name: &str, index: u32) -> Result<()> {
    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.header.flags = LinkFlags::Up; // IFF_UP = 1
    msg.header.change_mask = LinkFlags::Up; // change the UP flag

    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting link state to up : {name}"))?;
    Ok(())
}

/// Create a bridge interface
pub async fn create_bridge(name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Allow lldp
    //let mask: u16 = 0x4000;

    // Allow all protocols except for 00, 01, 02 -> TODO: Add doc link.
    let mask: u16 = 0xFFF8;

    handle
        .link()
        .add(LinkBridge::new(name).group_fwd_mask(mask).build())
        .execute()
        .await
        .context(format!("Error creating bridge: {name}"))?;

    let idx = get_link_index(&handle, name).await?;

    set_link_up(&handle, name, idx).await?;

    Ok(())
}

pub async fn create_veth_pair(src: &str, dst: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Create veth pair
    handle
        .link()
        .add(LinkVeth::new(src, dst).build())
        .execute()
        .await?;

    // bring src veth up
    let src_idx = get_link_index(&handle, src).await?;
    set_link_up(&handle, src, src_idx).await?;

    // bring dst veth up
    let dst_idx = get_link_index(&handle, dst).await?;
    set_link_up(&handle, dst, dst_idx).await?;

    Ok(())
}

/// Enslave a veth interface to a bridge
pub async fn enslave_to_bridge(veth_name: &str, bridge_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Get the veth interface
    let mut veth_links = handle
        .link()
        .get()
        .match_name(veth_name.to_string())
        .execute();
    let veth_link = if let Some(link) = veth_links.try_next().await? {
        link
    } else {
        return Err(anyhow!("Interface {} not found", veth_name));
    };

    // Get the bridge interface index
    let bridge_idx = get_link_index(&handle, bridge_name).await?;

    // Create a minimal LinkMessage for the change
    let mut change_msg = LinkMessage::default();
    change_msg.header.index = veth_link.header.index;
    change_msg
        .attributes
        .push(LinkAttribute::Controller(bridge_idx));

    handle
        .link()
        .change(change_msg)
        .execute()
        .await
        .context(format!(
            "Error setting veth as bridge slave.\n veth: {veth_name} - bridge: {bridge_name}"
        ))?;

    Ok(())
}

/// Delete and interface
pub async fn delete_interface(name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    // Get the interface index
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(link) = links.try_next().await? {
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

    let mut matching_interfaces = Vec::new();

    while let Some(link) = links.try_next().await? {
        // Extract the interface name from attributes
        for attr in link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                if name.contains(fuzzy_name) {
                    matching_interfaces.push(name);
                }
                break;
            }
        }
    }

    Ok(matching_interfaces)
}
