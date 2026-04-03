use anyhow::{Context, Result};
use aya::Ebpf;
use aya::maps::HashMap;
use aya::programs::tc::qdisc_detach_program;
use aya::programs::{SchedClassifier, TcAttachType};
use tracing::instrument;

/// Wrapper to ensure include_bytes!() data is 8-byte aligned.
/// The ELF parser requires naturally-aligned data, but include_bytes!()
/// only guarantees 1-byte alignment.
#[repr(C, align(8))]
struct AlignedElf<const N: usize>([u8; N]);

/// Pre-built eBPF redirect program ELF, embedded with 8-byte alignment.
static EBPF_REDIRECT_ELF: &AlignedElf<
    { include_bytes!("../../ebpf-redirect/ebpf-redirect.elf").len() },
> = &AlignedElf(*include_bytes!("../../ebpf-redirect/ebpf-redirect.elf"));

/// Attach a P2p redirect program to an interface.
///
/// Loads the eBPF TC classifier program, sets the peer interface index
/// in the BPF map, and attaches the program to the interface's ingress.
/// Packets arriving on `iface_name` will be redirected to `peer_ifindex`'s egress.
///
/// The Ebpf object is intentionally leaked so the TC filter, BPF program, and maps
/// persist in the kernel after this function returns. Cleanup happens when the
/// interface is deleted (e.g. VM destroy removes the tap device).
#[instrument(fields(%iface_name, peer_ifindex))]
pub fn attach_p2p_redirect(iface_name: &str, peer_ifindex: u32) -> Result<()> {
    // Detach any existing p2p_redirect program on this interface.
    // This makes the function idempotent for redeploy/resume scenarios.
    match qdisc_detach_program(iface_name, TcAttachType::Ingress, "p2p_redirect") {
        Ok(()) => {
            tracing::debug!(interface = %iface_name, "detached existing p2p_redirect program");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
            .program_mut("p2p_redirect")
            .context("p2p_redirect program not found in eBPF ELF")?
            .try_into()
            .context("failed to convert to SchedClassifier")?;

        program
            .load()
            .context(format!("failed to load TC program for {iface_name}"))?;

        program
            .attach(iface_name, TcAttachType::Ingress)
            .context(format!("failed to attach TC program to {iface_name}"))?;
    }

    // Intentionally leak the Ebpf object so the TC filter, BPF program, and maps
    // persist in the kernel after this function returns. The kernel reference-counts
    // these objects — they stay alive as long as the TC filter is attached.
    // Cleanup happens automatically when the tap interface is deleted (VM destroy).
    std::mem::forget(bpf);

    tracing::info!(
        interface = %iface_name,
        peer_ifindex = peer_ifindex,
        "attached eBPF p2p redirect program"
    );

    Ok(())
}
