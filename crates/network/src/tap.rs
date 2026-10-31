use std::io::ErrorKind;

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{
    InfoData, InfoKind, InfoVeth, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
};
use shared::konst::{MTU_JUMBO_NET, NETNS_VETH_INDEX_ATTEMPTS};
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

/// Create a veth pair whose far end is created straight into the network namespace of `pid`
/// (a container), named `netns_name`. The host end, `host_name`, stays in this namespace.
///
/// An enabled pair comes up with the far end promiscuous; a disabled one stays down on both
/// ends, so the far end has no carrier. Nothing runs inside the namespace, and no privilege
/// beyond CAP_NET_ADMIN is needed: the kernel brings up a new veth's own end only once both
/// ends exist, so the far end is the request's own end and the host end its peer.
///
/// The far end is given an interface index as well as its name. Older kernels, 6.8 among
/// them, look a new link's name up in this namespace first, so a host interface named like it
/// (`eth1`) would fail the request. Given an index, the kernel looks that up instead. Indexes
/// are per namespace, so one free here can be taken in the container's; the next is tried then.
#[instrument(fields(%host_name, %netns_name, pid, enabled), level = "debug")]
pub async fn create_veth_in_netns(
    host_name: &str,
    host_alias: &str,
    netns_name: &str,
    pid: u32,
    enabled: bool,
) -> Result<()> {
    let handle = setup_netlink().await?;
    let mtu = MTU_JUMBO_NET as u32;

    // A taken host end name would look like a taken index below.
    if get_link_index(&handle, host_name).await.is_ok() {
        bail!("cannot create veth {host_name}: an interface with that name already exists");
    }
    let mut links = handle.link().get().execute();
    let mut highest_index = 0;
    while let Some(link) = links
        .try_next()
        .await
        .context("failed to list interfaces")?
    {
        highest_index = highest_index.max(link.header.index);
    }

    let mut host_end = LinkMessage::default();
    host_end.attributes.extend(vec![
        LinkAttribute::IfName(host_name.to_string()),
        LinkAttribute::Mtu(mtu),
    ]);

    let mut netns_end = LinkMessage::default();
    if enabled {
        netns_end.header.flags = LinkFlags::Up | LinkFlags::Promisc;
        netns_end.header.change_mask = LinkFlags::Up | LinkFlags::Promisc;
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
        enabled = enabled,
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
    if enabled {
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
