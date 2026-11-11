use std::io::ErrorKind;

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{
    InfoData, InfoKind, InfoVeth, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
};
use rtnetlink::{Error, Handle};
use shared::konst::{MTU_JUMBO_NET, NETNS_VETH_INDEX_ATTEMPTS};
use tracing::instrument;

use crate::linux::{find_link_index, get_link_index, set_link_properties, setup_netlink};

/// How the two ends of a veth into a container come up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetnsVethState {
    /// Both ends up and the container end promiscuous: a connected interface.
    Enabled,
    /// Both ends down: a disabled interface, down in the container and with no carrier.
    Disabled,
    /// The container end up and the host end down: a disabled interface that the container
    /// sees up, with no carrier. For images that wait until every interface is up.
    DisabledUp,
}

/// Create a persistent tap device.
///
/// The tap device is created in UP state with jumbo MTU.
/// It can be used by libvirt via `<interface type='ethernet'><target dev='name'/>`
/// or by eBPF programs for packet redirection.
///
/// # Errors
///
/// Fails when netlink cannot create the device or set it up.
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
    set_link_properties(&handle, name, idx, alias_name, u32::from(MTU_JUMBO_NET)).await?;

    Ok(())
}

/// Create a veth pair whose far end is created straight into the network namespace of `pid`
/// (a container), named `netns_name`. The host end, `host_name`, stays in this namespace.
///
/// `state` says which ends come up. The far end has carrier only while the host end is up.
/// Nothing runs inside the namespace, and no privilege beyond `CAP_NET_ADMIN` is needed: the
/// kernel brings up a new veth's own end only once both ends exist, so the far end is the
/// request's own end and the host end its peer.
///
/// The far end is given an interface index as well as its name. Older kernels, 6.8 among
/// them, look a new link's name up in this namespace first, so a host interface named like it
/// (`eth1`) would fail the request. Given an index, the kernel looks that up instead. Indexes
/// are per namespace, so one free here can be taken in the container's; the next is tried then.
///
/// A veth named `host_name` whose peer is in another namespace is deleted first, with its peer:
/// it is the host end of a removed container's interface. The kernel tears a removed
/// container's namespace down in the background, at times minutes later, and its veths last
/// until then. Any other interface named `host_name` fails the request.
///
/// # Errors
///
/// Fails when `host_name` is taken by another kind of interface, no free interface index is
/// found, or netlink refuses to create or set up the pair.
#[instrument(fields(%host_name, %netns_name, pid, ?state), level = "debug")]
pub async fn create_veth_in_netns(
    host_name: &str,
    host_alias: &str,
    netns_name: &str,
    pid: u32,
    state: NetnsVethState,
) -> Result<()> {
    let handle = setup_netlink().await?;
    let mtu = u32::from(MTU_JUMBO_NET);

    let mut links = handle.link().get().execute();
    let mut highest_index = 0;
    let mut taken = None;
    while let Some(link) = links
        .try_next()
        .await
        .context("failed to list interfaces")?
    {
        highest_index = highest_index.max(link.header.index);
        if link
            .attributes
            .contains(&LinkAttribute::IfName(host_name.to_string()))
        {
            taken = Some(link);
        }
    }
    // A taken host end name would look like a taken index below.
    if let Some(link) = taken {
        if !is_container_veth(&link) {
            bail!("cannot create veth {host_name}: an interface with that name already exists");
        }
        tracing::warn!(
            host_veth = %host_name,
            "deleting a stale veth left by a removed container"
        );
        handle
            .link()
            .del(link.header.index)
            .execute()
            .await
            .context(format!("failed to delete stale veth {host_name}"))?;
    }

    let mut host_end = LinkMessage::default();
    host_end.attributes.extend(vec![
        LinkAttribute::IfName(host_name.to_string()),
        LinkAttribute::Mtu(mtu),
    ]);

    let mut netns_end = LinkMessage::default();
    match state {
        NetnsVethState::Enabled => {
            netns_end.header.flags = LinkFlags::Up | LinkFlags::Promisc;
            netns_end.header.change_mask = LinkFlags::Up | LinkFlags::Promisc;
        }
        NetnsVethState::DisabledUp => {
            netns_end.header.flags = LinkFlags::Up;
            netns_end.header.change_mask = LinkFlags::Up;
        }
        NetnsVethState::Disabled => {
            tracing::debug!(interface = %netns_name, "container end stays down");
        }
    }
    netns_end.attributes.extend(vec![
        LinkAttribute::IfName(netns_name.to_string()),
        LinkAttribute::NetNsPid(pid),
        LinkAttribute::Mtu(mtu),
        LinkAttribute::LinkInfo(vec![
            LinkInfo::Kind(InfoKind::Veth),
            LinkInfo::Data(InfoData::Veth(InfoVeth::Peer(host_end))),
        ]),
    ]);

    tracing::info!(
        host_veth = %host_name,
        interface = %netns_name,
        pid = pid,
        state = ?state,
        "creating veth pair into network namespace"
    );
    add_at_free_index(
        &handle,
        &mut netns_end,
        highest_index,
        &format!("{host_name} <--> {netns_name} in netns of pid {pid}"),
    )
    .await?;

    let index = get_link_index(&handle, host_name).await?;
    let mut msg = LinkMessage::default();
    msg.header.index = index;
    if state == NetnsVethState::Enabled {
        msg.header.flags = LinkFlags::Up;
        msg.header.change_mask = LinkFlags::Up;
    }
    msg.attributes
        .push(LinkAttribute::IfAlias(host_alias.to_string()));
    if let Err(e) = handle.link().set(msg).execute().await {
        // Deleting the host end deletes the far end too, so no half-made pair is left.
        if let Err(del) = handle.link().del(index).execute().await {
            tracing::warn!(host_veth = %host_name, error = %del, "failed to delete veth pair");
        }
        return Err(e).context(format!("failed to set up host veth {host_name}"));
    }

    Ok(())
}

/// Create the veth pair from its far end, `netns_end`, at the first free interface index above
/// `highest_index`. `pair` describes the pair in errors.
async fn add_at_free_index(
    handle: &Handle,
    netns_end: &mut LinkMessage,
    highest_index: u32,
    pair: &str,
) -> Result<()> {
    for index in highest_index + 1..=highest_index + NETNS_VETH_INDEX_ATTEMPTS {
        netns_end.header.index = index;
        match handle.link().add(netns_end.clone()).execute().await {
            Ok(()) => return Ok(()),
            Err(Error::NetlinkError(message))
                if matches!(
                    message.to_io().kind(),
                    ErrorKind::AlreadyExists | ErrorKind::ResourceBusy
                ) =>
            {
                tracing::debug!(index = index, "interface index taken, trying the next");
            }
            Err(e) => {
                return Err(e).context(format!("failed to create veth {pair}"));
            }
        }
    }
    bail!(
        "failed to create veth {pair}: no free interface index after {NETNS_VETH_INDEX_ATTEMPTS} tries"
    )
}

/// Get the interface index (ifindex) for a named interface.
///
/// # Errors
///
/// Fails when the interface does not exist or netlink cannot look it up.
#[instrument(fields(%iface_name), level = "debug")]
pub async fn get_ifindex(iface_name: &str) -> Result<u32> {
    let handle = setup_netlink().await?;
    get_link_index(&handle, iface_name).await
}

/// The interface index of `iface_name`, or `None` when there is no such interface: a link end
/// whose VM is shut off, or whose container's network namespace is gone.
///
/// # Errors
///
/// Fails when netlink cannot look the interface up.
#[instrument(fields(%iface_name), level = "debug")]
pub async fn find_ifindex(iface_name: &str) -> Result<Option<u32>> {
    let handle = setup_netlink().await?;
    find_link_index(&handle, iface_name).await
}

/// Whether `link` is a veth whose peer is in another network namespace: the host end of a
/// container interface.
fn is_container_veth(link: &LinkMessage) -> bool {
    let mut veth = false;
    let mut peer_elsewhere = false;
    for attribute in &link.attributes {
        if let LinkAttribute::LinkInfo(info) = attribute {
            veth = info.contains(&LinkInfo::Kind(InfoKind::Veth));
        }
        if let LinkAttribute::LinkNetNsId(_) = attribute {
            peer_elsewhere = true;
        }
    }
    veth && peer_elsewhere
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(attributes: Vec<LinkAttribute>) -> LinkMessage {
        let mut link = LinkMessage::default();
        link.attributes = attributes;
        link
    }

    #[test]
    fn a_veth_with_its_peer_in_another_namespace_is_a_container_veth() {
        let veth = LinkAttribute::LinkInfo(vec![LinkInfo::Kind(InfoKind::Veth)]);
        assert!(is_container_veth(&link(vec![
            veth.clone(),
            LinkAttribute::LinkNetNsId(6)
        ])));
        assert!(!is_container_veth(&link(vec![veth])));
        assert!(!is_container_veth(&link(vec![
            LinkAttribute::LinkInfo(vec![LinkInfo::Kind(InfoKind::Tun)]),
            LinkAttribute::LinkNetNsId(6),
        ])));
    }
}
