use std::io::ErrorKind;

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{
    InfoData, InfoKind, InfoVeth, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
};
use shared::konst::{MTU_JUMBO_NET, NETNS_VETH_INDEX_ATTEMPTS};
use tracing::instrument;

use crate::linux::{get_link_index, set_link_properties, setup_netlink};

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

/// Create a veth pair whose far end is created straight into the network namespace of `pid`
/// (a container), named `netns_name`. The host end, `host_name`, stays in this namespace.
///
/// `state` says which ends come up. The far end has carrier only while the host end is up. Nothing runs inside the namespace, and no privilege
/// beyond CAP_NET_ADMIN is needed: the kernel brings up a new veth's own end only once both
/// ends exist, so the far end is the request's own end and the host end its peer.
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
#[instrument(fields(%host_name, %netns_name, pid, ?state), level = "debug")]
pub async fn create_veth_in_netns(
    host_name: &str,
    host_alias: &str,
    netns_name: &str,
    pid: u32,
    state: NetnsVethState,
) -> Result<()> {
    let handle = setup_netlink().await?;
    let mtu = MTU_JUMBO_NET as u32;

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
    let mut created = false;
    for index in highest_index + 1..=highest_index + NETNS_VETH_INDEX_ATTEMPTS {
        netns_end.header.index = index;
        match handle.link().add(netns_end.clone()).execute().await {
            Ok(()) => {
                created = true;
                break;
            }
            Err(rtnetlink::Error::NetlinkError(message))
                if matches!(
                    message.to_io().kind(),
                    ErrorKind::AlreadyExists | ErrorKind::ResourceBusy
                ) =>
            {
                tracing::debug!(index = index, "interface index taken, trying the next");
            }
            Err(e) => {
                return Err(e).context(format!(
                    "failed to create veth {host_name} <--> {netns_name} in netns of pid {pid}"
                ));
            }
        }
    }
    if !created {
        bail!(
            "failed to create veth {host_name} <--> {netns_name} in netns of pid {pid}: no free \
             interface index after {NETNS_VETH_INDEX_ATTEMPTS} tries"
        );
    }

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

/// Get the interface index (ifindex) for a named interface.
#[instrument(fields(%iface_name), level = "debug")]
pub async fn get_ifindex(iface_name: &str) -> Result<u32> {
    let handle = setup_netlink().await?;
    get_link_index(&handle, iface_name).await
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
