use anyhow::{Context, Result};
use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkInfo, LinkMessage};
use shared::konst::MTU_JUMBO_NET;
use tracing::instrument;

use crate::linux::{get_link_index, set_link_properties, setup_netlink};

/// Create a persistent tap device.
///
/// The tap device is created in UP state with jumbo MTU.
/// It can be used by libvirt via `<interface type='ethernet'><target dev='name'/>`
/// or by eBPF programs for packet redirection.
#[instrument(fields(%name, %alias_name))]
pub async fn create_tap(name: &str, alias_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    let mut msg = LinkMessage::default();
    msg.attributes.push(LinkAttribute::IfName(name.to_string()));
    msg.attributes
        .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(InfoKind::Tun)]));

    tracing::info!(tap_name = %name, alias = %alias_name, "creating tap device");
    handle
        .link()
        .add(msg)
        .execute()
        .await
        .context(format!("failed to create tap device: {name}"))?;

    let idx = get_link_index(&handle, name).await?;
    set_link_properties(&handle, name, idx, alias_name, MTU_JUMBO_NET as u32).await?;

    Ok(())
}

/// Move a network interface into a different network namespace by PID.
///
/// Used to move a veth end into a container's network namespace.
#[instrument(fields(%iface_name, pid), level = "debug")]
pub async fn move_to_netns(iface_name: &str, pid: u32) -> Result<()> {
    let handle = setup_netlink().await?;
    let idx = get_link_index(&handle, iface_name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = idx;
    msg.attributes.push(LinkAttribute::NetNsPid(pid));

    tracing::info!(
        interface = %iface_name,
        pid = pid,
        "moving interface to network namespace"
    );

    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("failed to move {iface_name} to netns of pid {pid}"))?;

    Ok(())
}

/// Get the interface index (ifindex) for a named interface.
#[instrument(fields(%iface_name), level = "debug")]
pub async fn get_ifindex(iface_name: &str) -> Result<u32> {
    let handle = setup_netlink().await?;
    get_link_index(&handle, iface_name).await
}
