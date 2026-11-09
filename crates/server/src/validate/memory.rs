use anyhow::{Result, bail};

use libvirt::HostMemory;
use shared::data::HostMemorySettings;

/// Memory one VM or unikernel takes on the server, or will take once started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryClaim {
    /// The node or domain, as messages name it.
    pub name: String,
    pub memory_mib: u64,
    /// The one host NUMA node its memory must stay on, if any.
    pub numa_node: Option<u32>,
}

/// Memory already promised to other guests.
#[derive(Clone, Copy, Debug)]
pub struct CommittedMemory<'a> {
    /// Running and paused domains, Sherpa's and others'.
    pub running: &'a [MemoryClaim],
    /// Nodes other operations have been admitted to start, that are not running yet.
    pub starting: &'a [MemoryClaim],
}

/// Check that `requested` fits on the server beside what is `committed`: on the whole server,
/// and on each NUMA node a requested node's memory is held to.
///
/// Guests may have the server's memory, less its hugepage pools and the host reserve, times the
/// overcommit ratio. A NUMA node's guests may have its memory, less its hugepages and its share
/// of the reserve, times the ratio; only memory held strictly on that node counts against it,
/// because memory that is not bound may be placed on any node.
///
/// # Errors
///
/// Fails when the nodes do not fit, naming the numbers that fell short and the settings that
/// change them, or when a requested NUMA node does not exist.
pub fn check_memory_fits(
    requested: &[MemoryClaim],
    committed: CommittedMemory<'_>,
    host: &HostMemory,
    settings: &HostMemorySettings,
) -> Result<()> {
    if requested.is_empty() {
        return Ok(());
    }
    let ratio = settings.overcommit_ratio;
    let total_mib = host.total_kib / 1024;
    let hugepages_mib = host.hugepages_kib.div_ceil(1024);
    let reserve_mib = u64::from(settings.reserve_mib);
    let limit = scale_mib(
        total_mib
            .saturating_sub(hugepages_mib)
            .saturating_sub(reserve_mib),
        ratio,
    );
    let running = total(committed.running);
    let starting = total(committed.starting);
    let needed = total(requested);
    let left = limit.saturating_sub(running + starting);
    if needed > left {
        bail!(
            "Not enough memory on the server: {} {needed} MiB, but only {left} MiB of guest \
             memory is left, {} MiB short. The server has {total_mib} MiB; less \
             {hugepages_mib} MiB in hugepages and the {reserve_mib} MiB host reserve, at an \
             overcommit ratio of {ratio}, guests may have {limit} MiB. Running VMs and \
             unikernels hold {running} MiB ({}), and nodes other operations are starting hold \
             {starting} MiB ({}). Stop or destroy nodes to free memory, or an admin can lower \
             `host_memory.reserve_mib` or raise `host_memory.overcommit_ratio` in sherpa.toml.",
            needing(requested),
            needed - left,
            committed.running.len(),
            committed.starting.len(),
        );
    }

    let mut numa_nodes: Vec<u32> = requested.iter().filter_map(|c| c.numa_node).collect();
    numa_nodes.sort_unstable();
    numa_nodes.dedup();
    for numa_node in numa_nodes {
        check_numa_node_fits(numa_node, requested, committed, host, settings)?;
    }
    Ok(())
}

/// Check that the memory requested on `numa_node` fits beside the memory held there.
fn check_numa_node_fits(
    numa_node: u32,
    requested: &[MemoryClaim],
    committed: CommittedMemory<'_>,
    host: &HostMemory,
    settings: &HostMemorySettings,
) -> Result<()> {
    let Some(cell) = host.cells.iter().find(|cell| cell.id == numa_node) else {
        bail!("The server has no NUMA node {numa_node}");
    };
    let ratio = settings.overcommit_ratio;
    let cell_mib = cell.total_kib / 1024;
    let hugepages_mib = cell.hugepages_kib.div_ceil(1024);
    let host_mib = (host.total_kib / 1024).max(1);
    // The reserve is shared between the nodes in proportion to their memory.
    let reserve_share = u64::from(settings.reserve_mib).saturating_mul(cell_mib) / host_mib;
    let limit = scale_mib(
        cell_mib
            .saturating_sub(hugepages_mib)
            .saturating_sub(reserve_share),
        ratio,
    );
    let on_node = |claims: &[MemoryClaim]| -> Vec<MemoryClaim> {
        claims
            .iter()
            .filter(|claim| claim.numa_node == Some(numa_node))
            .cloned()
            .collect()
    };
    let wanted = on_node(requested);
    let mut holders = on_node(committed.running);
    holders.extend(on_node(committed.starting));
    let held = total(&holders);
    let needed = total(&wanted);
    let left = limit.saturating_sub(held);
    if needed > left {
        let holder_list = if holders.is_empty() {
            "none".to_string()
        } else {
            holders
                .iter()
                .map(|claim| format!("{} ({} MiB)", claim.name, claim.memory_mib))
                .collect::<Vec<_>>()
                .join(", ")
        };
        bail!(
            "Not enough memory on NUMA node {numa_node} for nodes held there with \
             `numa_node = {numa_node}`: {} {needed} MiB, but only {left} MiB is left, {} MiB \
             short. NUMA node {numa_node} has {cell_mib} MiB; less {hugepages_mib} MiB in \
             hugepages and its {reserve_share} MiB share of the host reserve, at an overcommit \
             ratio of {ratio}, guests may have {limit} MiB. VMs held strictly on NUMA node \
             {numa_node} hold {held} MiB: {holder_list}. Memory held on one NUMA node cannot \
             use another node's. Choose another `numa_node`, stop or destroy those nodes, or an \
             admin can lower `host_memory.reserve_mib` or raise `host_memory.overcommit_ratio` \
             in sherpa.toml.",
            needing(&wanted),
            needed - left,
        );
    }
    Ok(())
}

/// Check the `[host_memory]` settings.
///
/// # Errors
///
/// Fails when the overcommit ratio is not a finite number above zero.
pub fn check_host_memory_settings(settings: &HostMemorySettings) -> Result<()> {
    let ratio = settings.overcommit_ratio;
    if !ratio.is_finite() || ratio <= 0.0 {
        bail!("`host_memory.overcommit_ratio` must be a number above 0, not {ratio}");
    }
    Ok(())
}

/// The memory of every claim, in MiB.
fn total(claims: &[MemoryClaim]) -> u64 {
    claims
        .iter()
        .map(|claim| claim.memory_mib)
        .fold(0, u64::saturating_add)
}

/// "node 'a' needs" or "nodes 'a', 'b' need".
fn needing(claims: &[MemoryClaim]) -> String {
    let names = claims
        .iter()
        .map(|claim| format!("'{}'", claim.name))
        .collect::<Vec<_>>()
        .join(", ");
    if claims.len() == 1 {
        format!("node {names} needs")
    } else {
        format!("nodes {names} need")
    }
}

/// `mib` times `ratio`, rounded down. Memory beyond `u32::MAX` MiB (4 PiB) counts as that.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the product is clamped to 0..u64::MAX first; Rust has no checked float to int cast"
)]
fn scale_mib(mib: u64, ratio: f64) -> u64 {
    let scaled = f64::from(u32::try_from(mib).unwrap_or(u32::MAX)) * ratio;
    if scaled.is_nan() || scaled <= 0.0 {
        return 0;
    }
    if scaled >= 2f64.powi(64) {
        return u64::MAX;
    }
    scaled as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use libvirt::NumaCell;

    const GIB: u64 = 1024;

    /// Two NUMA nodes of 80 GiB, as on the dev server.
    fn host() -> HostMemory {
        HostMemory {
            total_kib: 160 * GIB * 1024,
            hugepages_kib: 0,
            cells: vec![
                NumaCell {
                    id: 0,
                    total_kib: 80 * GIB * 1024,
                    hugepages_kib: 0,
                },
                NumaCell {
                    id: 1,
                    total_kib: 80 * GIB * 1024,
                    hugepages_kib: 0,
                },
            ],
        }
    }

    fn settings(reserve_mib: u32, overcommit_ratio: f64) -> HostMemorySettings {
        HostMemorySettings {
            reserve_mib,
            overcommit_ratio,
        }
    }

    fn claim(name: &str, gib: u64, numa_node: Option<u32>) -> MemoryClaim {
        MemoryClaim {
            name: name.to_string(),
            memory_mib: gib * GIB,
            numa_node,
        }
    }

    fn check(
        requested: &[MemoryClaim],
        running: &[MemoryClaim],
        starting: &[MemoryClaim],
        settings: &HostMemorySettings,
    ) -> Result<()> {
        check_memory_fits(
            requested,
            CommittedMemory { running, starting },
            &host(),
            settings,
        )
    }

    fn error(result: Result<()>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn nodes_that_fit_beside_the_running_ones_are_admitted() {
        let running = [claim("dev", 48, Some(1)), claim("rel2404", 30, None)];
        let requested = [claim("rel2604", 30, None), claim("agent", 24, Some(0))];
        check(&requested, &running, &[], &settings(4096, 1.0)).unwrap();
    }

    #[test]
    fn the_global_oom_of_514_is_refused_naming_every_number_and_the_settings() {
        // 148 GiB of guests on a 160 GiB server leaves less than the 16 GiB reserve.
        let running = [
            claim("dev", 48, Some(1)),
            claim("agent", 24, Some(0)),
            claim("rel2404", 30, None),
            claim("win", 16, None),
        ];
        let requested = [claim("rel2604", 30, None)];
        let message = error(check(&requested, &running, &[], &settings(16 * 1024, 1.0)));
        for expected in [
            "Not enough memory on the server",
            "node 'rel2604' needs 30720 MiB",
            "only 26624 MiB",
            "4096 MiB short",
            "The server has 163840 MiB",
            "16384 MiB host reserve",
            "overcommit ratio of 1",
            "guests may have 147456 MiB",
            "Running VMs and unikernels hold 120832 MiB (4)",
            "`host_memory.reserve_mib`",
            "`host_memory.overcommit_ratio`",
            "sherpa.toml",
        ] {
            assert!(message.contains(expected), "{expected:?} not in: {message}");
        }
    }

    #[test]
    fn nodes_other_operations_are_starting_count_too() {
        let starting = [claim("other-lab", 100, None)];
        let requested = [claim("a", 40, None), claim("b", 20, None)];
        let message = error(check(&requested, &[], &starting, &settings(4096, 1.0)));
        assert!(
            message.contains("nodes 'a', 'b' need 61440 MiB"),
            "{message}"
        );
        assert!(
            message.contains("nodes other operations are starting hold 102400 MiB (1)"),
            "{message}"
        );
    }

    #[test]
    fn the_overcommit_ratio_scales_what_guests_may_have() {
        let running = [claim("big", 150, None)];
        let requested = [claim("more", 60, None)];
        assert!(check(&requested, &running, &[], &settings(4096, 1.0)).is_err());
        check(&requested, &running, &[], &settings(4096, 1.5)).unwrap();
    }

    #[test]
    fn hugepage_pools_are_not_guest_memory() {
        let mut host = host();
        host.hugepages_kib = 40 * GIB * 1024;
        host.cells[0].hugepages_kib = 40 * GIB * 1024;
        let requested = [claim("a", 130, None)];
        let committed = CommittedMemory {
            running: &[],
            starting: &[],
        };
        let message = error(check_memory_fits(
            &requested,
            committed,
            &host,
            &settings(0, 1.0),
        ));
        assert!(message.contains("less 40960 MiB in hugepages"), "{message}");
    }

    #[test]
    fn the_numa_node_oom_of_514_is_refused_naming_the_node_and_its_holders() {
        // The whole server has room, but NUMA node 0 does not.
        let running = [claim("agent", 60, Some(0)), claim("dev", 10, Some(1))];
        let requested = [claim("pinned", 24, Some(0))];
        let message = error(check(&requested, &running, &[], &settings(4096, 1.0)));
        for expected in [
            "Not enough memory on NUMA node 0",
            "`numa_node = 0`",
            "node 'pinned' needs 24576 MiB",
            "NUMA node 0 has 81920 MiB",
            "2048 MiB share of the host reserve",
            "VMs held strictly on NUMA node 0 hold 61440 MiB: agent (61440 MiB)",
            "`host_memory.reserve_mib`",
        ] {
            assert!(message.contains(expected), "{expected:?} not in: {message}");
        }
    }

    #[test]
    fn unbound_memory_only_counts_against_the_whole_server() {
        let running = [claim("unpinned", 70, None)];
        let requested = [claim("pinned", 70, Some(0))];
        check(&requested, &running, &[], &settings(4096, 1.0)).unwrap();
    }

    #[test]
    fn several_requested_nodes_on_one_numa_node_add_up() {
        let requested = [claim("a", 40, Some(1)), claim("b", 40, Some(1))];
        let message = error(check(&requested, &[], &[], &settings(4096, 1.0)));
        assert!(
            message.contains("Not enough memory on NUMA node 1"),
            "{message}"
        );
        assert!(
            message.contains("nodes 'a', 'b' need 81920 MiB"),
            "{message}"
        );
    }

    #[test]
    fn a_numa_node_the_server_does_not_have_is_refused() {
        let requested = [claim("a", 1, Some(7))];
        let message = error(check(&requested, &[], &[], &settings(4096, 1.0)));
        assert!(message.contains("NUMA node 7"), "{message}");
    }

    #[test]
    fn nothing_requested_always_fits() {
        let running = [claim("huge", 500, None)];
        check(&[], &running, &[], &settings(4096, 1.0)).unwrap();
    }

    #[test]
    fn the_overcommit_ratio_must_be_a_number_above_zero() {
        check_host_memory_settings(&settings(4096, 1.0)).unwrap();
        check_host_memory_settings(&settings(0, 2.5)).unwrap();
        for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let message = error(check_host_memory_settings(&settings(4096, ratio)));
            assert!(
                message.contains("`host_memory.overcommit_ratio`"),
                "{message}"
            );
        }
    }
}
