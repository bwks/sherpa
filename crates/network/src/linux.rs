use anyhow::{Context, Result, bail};
use futures::{StreamExt, TryStreamExt};
use rtnetlink::packet_core::{NLM_F_ACK, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::packet_route::nsid::{NsidAttribute, NsidMessage};
use rtnetlink::{Error, Handle, LinkBridge, LinkVeth, new_connection};
use shared::konst::MTU_JUMBO_NET;
use tracing::instrument;

/// Linux's `ENODEV`: netlink's answer for a link name that does not exist.
const ENODEV: i32 = 19;

/// The link-local group addresses a bridge forwards, by the last nibble of
/// 01-80-C2-00-00-0X.
///
/// IEEE 802.1D bridges do not relay frames sent to 01-80-C2-00-00-00 to 01-80-C2-00-00-0F.
/// This forwards all of them except 00, 01 and 02, which the kernel refuses to forward:
/// Spanning Tree (STP/RSTP/MSTP), Ethernet flow control (pause frames) and LACP.
/// See <https://interestingtraffic.nl/2017/11/21/an-oddly-specific-post-about-group_fwd_mask/>.
const BRIDGE_GROUP_FWD_MASK: u16 = 0xFFF8;

/// Open a netlink connection, driven by a task on the current runtime.
pub(crate) async fn setup_netlink() -> Result<Handle> {
    let (connection, handle, _) = new_connection().context("Error creating netlink connection")?;
    tokio::spawn(connection);
    Ok(handle)
}

/// Whether `error` is netlink's "no such device".
fn is_no_device(error: &Error) -> bool {
    matches!(error, Error::NetlinkError(message) if message.to_io().raw_os_error() == Some(ENODEV))
}

/// The index of the link `name`, or `None` when there is no such link.
pub(crate) async fn find_link_index(handle: &Handle, name: &str) -> Result<Option<u32>> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    match links.try_next().await {
        Ok(link) => Ok(link.map(|link| link.header.index)),
        Err(error) if is_no_device(&error) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to look up link {name}")),
    }
}

/// The index of the link `name`.
pub(crate) async fn get_link_index(handle: &Handle, name: &str) -> Result<u32> {
    find_link_index(handle, name)
        .await?
        .with_context(|| format!("Link {name} not found"))
}

/// Send `message` and return the first answer: the message the kernel sent back, or `None` for
/// an acknowledgement.
async fn request_one(
    handle: &mut Handle,
    message: RouteNetlinkMessage,
    flags: u16,
) -> Result<Option<RouteNetlinkMessage>> {
    let mut request = NetlinkMessage::from(message);
    request.header.flags = flags;
    let mut responses = handle
        .request(request)
        .context("Failed to send a netlink request")?;
    while let Some(response) = responses.next().await {
        match response.payload {
            NetlinkPayload::InnerMessage(answer) => return Ok(Some(answer)),
            NetlinkPayload::Error(error) if error.code.is_none() => return Ok(None),
            NetlinkPayload::Error(error) => {
                return Err(Error::NetlinkError(error)).context("Netlink refused the request");
            }
            _ => {}
        }
    }
    bail!("Netlink sent no answer")
}

/// The ID this network namespace has for the network namespace of process `pid`, or `None`
/// when it has none yet.
async fn netns_id_of_pid(handle: &mut Handle, pid: u32) -> Result<Option<i32>> {
    let mut message = NsidMessage::default();
    message.attributes.push(NsidAttribute::Pid(pid));
    let answer = request_one(handle, RouteNetlinkMessage::GetNsId(message), NLM_F_REQUEST)
        .await
        .with_context(|| format!("Failed to look up the network namespace of process {pid}"))?;
    let Some(RouteNetlinkMessage::NewNsId(reply)) = answer else {
        bail!("Unexpected answer to the namespace lookup of process {pid}");
    };
    let id = reply
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            NsidAttribute::Id(id) => Some(*id),
            _ => None,
        });
    // -1: not assigned.
    Ok(id.filter(|id| *id >= 0))
}

/// The ID this network namespace has for the network namespace of process `pid`, assigning
/// one when it has none.
async fn netns_id(handle: &mut Handle, pid: u32) -> Result<i32> {
    if let Some(id) = netns_id_of_pid(handle, pid).await? {
        return Ok(id);
    }
    let mut message = NsidMessage::default();
    message.attributes.push(NsidAttribute::Pid(pid));
    message.attributes.push(NsidAttribute::Id(-1));
    request_one(
        handle,
        RouteNetlinkMessage::NewNsId(message),
        NLM_F_REQUEST | NLM_F_ACK,
    )
    .await
    .with_context(|| format!("Failed to name the network namespace of process {pid}"))?;
    netns_id_of_pid(handle, pid)
        .await?
        .with_context(|| format!("The network namespace of process {pid} has no ID"))
}

/// The index, in this network namespace, of the veth peer of interface `iface` of process
/// `pid`: the host end of a container's veth. Read through netlink with the namespace's ID,
/// which needs `CAP_NET_ADMIN` only.
///
/// # Errors
///
/// Fails when the process, its interface or the interface's peer does not exist.
#[instrument(level = "debug")]
pub async fn container_veth_peer_ifindex(pid: u32, iface: &str) -> Result<u32> {
    let mut handle = setup_netlink().await?;
    let nsid = netns_id(&mut handle, pid).await?;
    let mut request = handle.link().get().match_name(iface.to_string());
    request
        .message_mut()
        .attributes
        .push(LinkAttribute::IfNetnsId(nsid));
    let link = request
        .execute()
        .try_next()
        .await
        .with_context(|| format!("Failed to look up {iface} of process {pid}"))?
        .with_context(|| format!("Process {pid} has no interface {iface}"))?;
    link.attributes
        .iter()
        .find_map(|attribute| match attribute {
            LinkAttribute::Link(peer) => Some(*peer),
            _ => None,
        })
        .with_context(|| format!("{iface} of process {pid} has no peer"))
}

/// Set the admin state of link `name`: up, or down.
///
/// Setting a veth end down takes the carrier from its peer, which is how a disabled container
/// interface, or the peer of a P2p link end whose neighbour is down, loses its link.
///
/// # Errors
///
/// Fails when the link does not exist or netlink refuses the change.
#[instrument(fields(%name, up), level = "debug")]
pub async fn set_link_state(name: &str, up: bool) -> Result<()> {
    let handle = setup_netlink().await?;
    let index = get_link_index(&handle, name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.header.flags = if up {
        LinkFlags::Up
    } else {
        LinkFlags::empty()
    };
    msg.header.change_mask = LinkFlags::Up;

    tracing::debug!(link_name = %name, up, "Setting link state");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .with_context(|| format!("Error setting the state of link {name}"))?;
    Ok(())
}

/// Whether link `name` is administratively up, or `None` when there is no such link.
///
/// # Errors
///
/// Fails when netlink refuses the lookup.
#[instrument(fields(%name), level = "debug")]
pub async fn link_is_up(name: &str) -> Result<Option<bool>> {
    let handle = setup_netlink().await?;
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    match links.try_next().await {
        Ok(link) => Ok(link.map(|link| link.header.flags.contains(LinkFlags::Up))),
        Err(error) if is_no_device(&error) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to look up link {name}")),
    }
}

/// Bring a link up with an alias and an MTU.
pub(crate) async fn set_link_properties(
    handle: &Handle,
    link_name: &str,
    link_idx: u32,
    link_alias: &str,
    mtu: u32,
) -> Result<()> {
    let mut msg = LinkMessage::default();
    msg.header.index = link_idx;
    msg.header.flags = LinkFlags::Up;
    msg.header.change_mask = LinkFlags::Up;
    msg.attributes.extend([
        LinkAttribute::IfAlias(link_alias.to_string()),
        LinkAttribute::Mtu(mtu),
    ]);

    tracing::debug!(link_name = %link_name, alias = %link_alias, mtu = mtu, "Setting link properties");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting properties on: {link_name}"))?;

    Ok(())
}

/// Create a bridge that forwards link-local protocols such as LLDP, up, with a jumbo MTU.
///
/// # Errors
///
/// Fails when netlink cannot create the bridge or set it up.
#[instrument(fields(%name, %alias_name))]
pub async fn create_bridge(name: &str, alias_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;

    tracing::info!(bridge_name = %name, alias = %alias_name, "Creating bridge");
    handle
        .link()
        .add(
            LinkBridge::new(name)
                .group_fwd_mask(BRIDGE_GROUP_FWD_MASK)
                .build(),
        )
        .execute()
        .await
        .context(format!("Error creating bridge: {name}"))?;

    let idx = get_link_index(&handle, name).await?;

    set_link_properties(&handle, name, idx, alias_name, u32::from(MTU_JUMBO_NET)).await?;

    Ok(())
}

/// Create a veth pair, both ends up with a jumbo MTU and their aliases.
///
/// # Errors
///
/// Fails when netlink cannot create the pair or set its ends up.
#[instrument(fields(%src_name, %dst_name))]
pub async fn create_veth_pair(
    src_name: &str,
    dst_name: &str,
    src_alias_name: &str,
    dst_alias_name: &str,
) -> Result<()> {
    let handle = setup_netlink().await?;

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
    set_link_properties(
        &handle,
        src_name,
        src_idx,
        src_alias_name,
        u32::from(MTU_JUMBO_NET),
    )
    .await?;

    let dst_idx = get_link_index(&handle, dst_name).await?;
    set_link_properties(
        &handle,
        dst_name,
        dst_idx,
        dst_alias_name,
        u32::from(MTU_JUMBO_NET),
    )
    .await?;

    Ok(())
}

/// Make the interface `int_name` a member of the bridge `bridge_name`.
///
/// # Errors
///
/// Fails when either link does not exist or netlink refuses the change.
#[instrument(fields(%int_name, %bridge_name))]
pub async fn enslave_to_bridge(int_name: &str, bridge_name: &str) -> Result<()> {
    let handle = setup_netlink().await?;
    let index = get_link_index(&handle, int_name).await?;
    let bridge_idx = get_link_index(&handle, bridge_name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.attributes.push(LinkAttribute::Controller(bridge_idx));

    tracing::debug!(interface_name = %int_name, bridge_name = %bridge_name, "Enslaving interface to bridge");
    handle.link().change(msg).execute().await.context(format!(
        "Error setting interface {int_name} as a member of bridge {bridge_name}"
    ))?;

    Ok(())
}

/// Delete the interface `name`. An interface that is already gone counts as deleted, so a
/// retried destroy succeeds.
///
/// # Errors
///
/// Fails when netlink cannot look the interface up or delete it.
#[instrument(fields(%name))]
pub async fn delete_interface(name: &str) -> Result<()> {
    let handle = setup_netlink().await?;
    let Some(index) = find_link_index(&handle, name).await? else {
        tracing::debug!(interface_name = %name, "Interface already deleted");
        return Ok(());
    };

    tracing::info!(interface_name = %name, "Deleting interface");
    match handle.link().del(index).execute().await {
        Ok(()) => Ok(()),
        // Deleting a veth deletes its peer, so another delete can remove it first.
        Err(error) if is_no_device(&error) => {
            tracing::debug!(interface_name = %name, "Interface deleted meanwhile");
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("Error deleting interface {name}")),
    }
}

/// The names of every interface on the host.
///
/// # Errors
///
/// Fails when netlink cannot list the interfaces.
#[instrument(level = "debug")]
pub async fn list_interface_names() -> Result<Vec<String>> {
    let handle = setup_netlink().await?;

    let mut links = handle.link().get().execute();

    let mut interfaces = Vec::new();

    while let Some(link) = links.try_next().await.context("Error listing interfaces")? {
        for attr in link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                interfaces.push(name);
                break;
            }
        }
    }

    Ok(interfaces)
}
