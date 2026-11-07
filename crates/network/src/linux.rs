use anyhow::{Context, Result};
use futures::TryStreamExt;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
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
async fn find_link_index(handle: &Handle, name: &str) -> Result<Option<u32>> {
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

/// Set a link to DOWN state.
///
/// Used to remove carrier from a veth peer (e.g. disabled container interfaces).
///
/// # Errors
///
/// Fails when the link does not exist or netlink refuses the change.
#[instrument(fields(%name), level = "debug")]
pub async fn set_link_down(name: &str) -> Result<()> {
    let handle = setup_netlink().await?;
    let index = get_link_index(&handle, name).await?;

    let mut msg = LinkMessage::default();
    msg.header.index = index;
    msg.header.flags = LinkFlags::empty();
    msg.header.change_mask = LinkFlags::Up;

    tracing::debug!(link_name = %name, "Setting link down");
    handle
        .link()
        .set(msg)
        .execute()
        .await
        .context(format!("Error setting link down: {name}"))?;
    Ok(())
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
