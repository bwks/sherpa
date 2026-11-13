use std::io::ErrorKind;
use std::mem;

use anyhow::{Context, Result, anyhow};
use aya::Ebpf;
use aya::maps::HashMap;
use aya::programs::tc::{NlOptions, TcAttachOptions, qdisc_add_clsact, qdisc_detach_program};
use aya::programs::{SchedClassifier, TcAttachType};
use futures::StreamExt;
use rtnetlink::packet_core::{
    NLM_F_DUMP, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload, Nla, NlasIterator,
};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::tc::{TcAttribute, TcHandle, TcMessage, TcOption};
use shared::konst::TCA_BPF_NAME;
use tracing::instrument;

use crate::linux::setup_netlink;
use crate::tc::tc_index;

/// The name of the redirect program in the ELF, which its TC filter carries.
const REDIRECT_PROGRAM: &str = "p2p_redirect";

/// The parent of the filters on an interface's ingress hook: the `clsact` qdisc's ingress.
const CLSACT_INGRESS: TcHandle = TcHandle {
    major: TcHandle::CLSACT.major,
    minor: TcHandle::MIN_INGRESS,
};

/// Pre-built eBPF redirect program ELF, embedded with 8-byte alignment.
static EBPF_REDIRECT_ELF: &AlignedElf<
    { include_bytes!("../../ebpf-redirect/ebpf-redirect.elf").len() },
> = &AlignedElf(*include_bytes!("../../ebpf-redirect/ebpf-redirect.elf"));

/// Wrapper to ensure `include_bytes!()` data is 8-byte aligned.
/// The ELF parser requires naturally-aligned data, but `include_bytes!()`
/// only guarantees 1-byte alignment.
#[repr(C, align(8))]
struct AlignedElf<const N: usize>([u8; N]);

/// Attach a P2p redirect program to an interface.
///
/// Loads the eBPF TC classifier program, sets the peer interface index
/// in the BPF map, and attaches the program to the interface's ingress.
/// Packets arriving on `iface_name` will be redirected to `peer_ifindex`'s egress.
///
/// The program is attached as a legacy TC filter on the interface's `clsact` qdisc, never
/// through TCX, which aya picks by default on kernels >= 6.6. A TCX attachment lasts only as
/// long as sherpad holds its file descriptor, so a sherpad restart would drop it, and
/// `qdisc_detach_program` cannot remove it, so a re-attach would leave the old program in
/// front of the new one.
///
/// The TC filter holds the program, and the program its map, so both stay in the kernel after
/// this function returns and its file descriptors close. They go when the interface is deleted
/// (e.g. VM destroy removes the tap device).
///
/// # Errors
///
/// Fails when the clsact qdisc cannot be added, or the program cannot be loaded, given the peer
/// index or attached.
#[instrument(fields(%iface_name, peer_ifindex))]
pub fn attach_p2p_redirect(iface_name: &str, peer_ifindex: u32) -> Result<()> {
    // Legacy TC filters hang off the clsact qdisc; it exists already on a re-attach.
    match qdisc_add_clsact(iface_name) {
        Ok(()) => {
            tracing::debug!(interface = %iface_name, "added clsact qdisc");
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            tracing::debug!(interface = %iface_name, "clsact qdisc already present");
        }
        Err(e) => {
            return Err(e).context(format!("failed to add clsact qdisc to {iface_name}"));
        }
    }

    // Detach any existing p2p_redirect program on this interface.
    // This makes the function idempotent for redeploy/resume scenarios.
    match qdisc_detach_program(iface_name, TcAttachType::Ingress, REDIRECT_PROGRAM) {
        Ok(()) => {
            tracing::debug!(interface = %iface_name, "detached existing p2p_redirect program");
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // No existing program — expected on first attach
        }
        Err(e) => {
            tracing::warn!(
                interface = %iface_name,
                error = ?e,
                "failed to detach existing p2p_redirect, proceeding with attach"
            );
        }
    }

    let mut bpf =
        Ebpf::load(&EBPF_REDIRECT_ELF.0).context("failed to load eBPF redirect program")?;

    // Set the peer interface index in the BPF map
    {
        let mut peer_map: HashMap<_, u32, u32> = HashMap::try_from(
            bpf.map_mut("PEER_IFINDEX")
                .context("PEER_IFINDEX map not found in eBPF program")?,
        )
        .context("failed to create HashMap from PEER_IFINDEX map")?;

        peer_map
            .insert(0, peer_ifindex, 0)
            .context("failed to insert peer ifindex into BPF map")?;
    }

    // Load and attach the TC classifier program
    {
        let program: &mut SchedClassifier = bpf
            .program_mut(REDIRECT_PROGRAM)
            .context("p2p_redirect program not found in eBPF ELF")?
            .try_into()
            .context("failed to convert to SchedClassifier")?;

        program
            .load()
            .context(format!("failed to load TC program for {iface_name}"))?;

        let link_id = program
            .attach_with_options(
                iface_name,
                TcAttachType::Ingress,
                TcAttachOptions::Netlink(NlOptions::default()),
            )
            .context(format!("failed to attach TC program to {iface_name}"))?;

        // Dropping the program detaches every link it still holds, so the attachment is taken
        // out of it and forgotten. A legacy TC link holds no file descriptor; the program's and
        // the map's descriptors close when `bpf` is dropped, and the filter keeps both alive.
        let link = program
            .take_link(link_id)
            .context(format!("failed to take the TC attachment on {iface_name}"))?;
        mem::forget(link);
    }

    tracing::info!(
        interface = %iface_name,
        peer_ifindex = peer_ifindex,
        "attached eBPF p2p redirect program"
    );

    Ok(())
}

/// Whether the interface with index `iface_index` has the P2p redirect program on its ingress:
/// a link end that [`attach_p2p_redirect`] wired, and that has not been made again since. A
/// new interface never has it, as a TC filter cannot move between interfaces.
///
/// # Errors
///
/// Fails when the index is out of range, or netlink cannot list the interface's filters.
#[instrument(fields(iface_index), level = "debug")]
pub async fn has_p2p_redirect(iface_index: u32) -> Result<bool> {
    let mut message = TcMessage::default();
    message.header.index = tc_index(iface_index)?;
    message.header.parent = CLSACT_INGRESS;
    let mut request = NetlinkMessage::from(RouteNetlinkMessage::GetTrafficFilter(message));
    request.header.flags = NLM_F_REQUEST | NLM_F_DUMP;

    let mut handle = setup_netlink().await?;
    let mut response = handle
        .request(request)
        .context("failed to send the TC filter dump request")?;
    let mut found = false;
    while let Some(message) = response.next().await {
        match message.payload {
            NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewTrafficFilter(filter)) => {
                found |= is_p2p_redirect(&filter);
            }
            NetlinkPayload::Error(error) if error.code.is_some() => {
                return Err(anyhow!("TC netlink error: {error}")).with_context(|| {
                    format!("failed to list the ingress filters of ifindex {iface_index}")
                });
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Whether `filter`, from a filter dump, is a `bpf` filter running the P2p redirect program.
fn is_p2p_redirect(filter: &TcMessage) -> bool {
    let mut bpf = false;
    let mut named = false;
    for attribute in &filter.attributes {
        match attribute {
            TcAttribute::Kind(kind) => bpf = kind == "bpf",
            TcAttribute::Options(options) => named = options.iter().any(names_redirect),
            _ => {}
        }
    }
    bpf && named
}

/// Whether a `bpf` filter's options name the P2p redirect program. netlink-packet-route keeps
/// a `bpf` filter's options as one raw attribute, so the program's name (`TCA_BPF_NAME`, a
/// NUL-terminated string) is read from the attributes nested in it.
fn names_redirect(option: &TcOption) -> bool {
    let TcOption::Other(options) = option else {
        return false;
    };
    let mut nested = vec![0; options.value_len()];
    options.emit_value(&mut nested);
    NlasIterator::new(nested.as_slice())
        .filter_map(Result::ok)
        .any(|attribute| {
            attribute.kind() == TCA_BPF_NAME
                && attribute.value().strip_suffix(&[0]) == Some(REDIRECT_PROGRAM.as_bytes())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command;

    use rtnetlink::packet_core::{DefaultNla, Emitable};
    use shared::konst::TCA_OPTIONS;

    use crate::{create_veth_pair, delete_interface, get_ifindex};

    /// A filter of `kind` whose options carry a program named `name`, as the kernel dumps it.
    fn filter(kind: &str, name: &str) -> TcMessage {
        let mut program = name.as_bytes().to_vec();
        program.push(0);
        let name = DefaultNla::new(TCA_BPF_NAME, program);
        let mut nested = vec![0; name.buffer_len()];
        name.emit(&mut nested);
        let mut message = TcMessage::default();
        message.attributes.push(TcAttribute::Kind(kind.to_string()));
        message
            .attributes
            .push(TcAttribute::Options(vec![TcOption::Other(
                DefaultNla::new(TCA_OPTIONS, nested),
            )]));
        message
    }

    #[test]
    fn only_a_bpf_filter_running_the_redirect_program_is_the_redirect() {
        assert!(is_p2p_redirect(&filter("bpf", "p2p_redirect")));
        assert!(!is_p2p_redirect(&filter("bpf", "other_prog")));
        assert!(!is_p2p_redirect(&filter("bpf", "p2p_redirect_v2")));
        assert!(!is_p2p_redirect(&filter("u32", "p2p_redirect")));
        assert!(!is_p2p_redirect(&TcMessage::default()));
    }

    #[tokio::test]
    #[ignore = "requires root: makes a veth and attaches an eBPF program"]
    async fn a_new_interface_has_no_redirect_until_one_is_attached() {
        let (a, b) = ("it-hasred-a", "it-hasred-b");
        let _ = delete_interface(a).await;
        create_veth_pair(a, b, a, b).await.unwrap();
        let ifindex_a = get_ifindex(a).await.unwrap();
        let ifindex_b = get_ifindex(b).await.unwrap();

        assert!(!has_p2p_redirect(ifindex_a).await.unwrap(), "a bare end");
        let status = Command::new("tc")
            .args(["qdisc", "add", "dev", a, "clsact"])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            !has_p2p_redirect(ifindex_a).await.unwrap(),
            "a clsact qdisc with no filter"
        );

        attach_p2p_redirect(a, ifindex_b).unwrap();
        assert!(has_p2p_redirect(ifindex_a).await.unwrap());
        assert!(!has_p2p_redirect(ifindex_b).await.unwrap());

        delete_interface(a).await.unwrap();
    }
}
