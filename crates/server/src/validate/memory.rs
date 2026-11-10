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
/// overcommit percentage. A NUMA node's guests may have its memory, less its hugepages and its
/// share of the reserve, times the percentage. Only memory held strictly on that node counts
/// against it: memory that is not bound, or only prefers a node, may be placed on any node.
///
/// # Errors
///
/// Fails when the nodes do not fit, naming the numbers that fell short and the settings that
/// change them, or when a requested NUMA node does not exist.
pub fn check_memory_fits(
    requested: &[MemoryClaim],
    committed: CommittedMemory<'_>,
    host: &HostMemory,
    settings: HostMemorySettings,
) -> Result<()> {
    if requested.is_empty() {
        return Ok(());
    }
    let percent = settings.overcommit_percent;
    let total_mib = host.total_kib / 1024;
    let hugepages_mib = host.hugepages_kib.div_ceil(1024);
    let reserve_mib = u64::from(settings.reserve_mib);
    let limit = scale_mib(
        total_mib
            .saturating_sub(hugepages_mib)
            .saturating_sub(reserve_mib),
        percent,
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
             overcommit of {percent}%, guests may have {limit} MiB. Running VMs and \
             unikernels hold {running} MiB ({}), and nodes other operations are starting hold \
             {starting} MiB ({}). Stop or destroy nodes to free memory, or an admin can lower \
             `host_memory.reserve_mib` or raise `host_memory.overcommit_percent` in sherpa.toml.",
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
    settings: HostMemorySettings,
) -> Result<()> {
    let Some(cell) = host.cells.iter().find(|cell| cell.id == numa_node) else {
        bail!("The server has no NUMA node {numa_node}");
    };
    let percent = settings.overcommit_percent;
    let cell_mib = cell.total_kib / 1024;
    let hugepages_mib = cell.hugepages_kib.div_ceil(1024);
    let host_mib = (host.total_kib / 1024).max(1);
    // The reserve is shared between the nodes in proportion to their memory.
    let reserve_share = u64::from(settings.reserve_mib).saturating_mul(cell_mib) / host_mib;
    let limit = scale_mib(
        cell_mib
            .saturating_sub(hugepages_mib)
            .saturating_sub(reserve_share),
        percent,
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
             of {percent}%, guests may have {limit} MiB. VMs held strictly on NUMA node \
             {numa_node} hold {held} MiB: {holder_list}. Memory held on one NUMA node cannot \
             use another node's. Choose another `numa_node`, set \
             `numa_memory_mode = \"preferred\"` to let the node's memory spill to other NUMA \
             nodes, stop or destroy those nodes, or an admin can lower \
             `host_memory.reserve_mib` or raise `host_memory.overcommit_percent` in sherpa.toml.",
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
/// Fails when the overcommit percentage is 0.
pub fn check_host_memory_settings(settings: HostMemorySettings) -> Result<()> {
    let percent = settings.overcommit_percent;
    if percent < 1 {
        bail!("`host_memory.overcommit_percent` must be at least 1, not {percent}");
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

/// `percent`% of `mib`, rounded down, or `u64::MAX` when that does not fit.
///
/// `mib` is split into hundreds and a remainder below 100, so the result is exact without
/// multiplying all of `mib` first.
fn scale_mib(mib: u64, percent: u32) -> u64 {
    let percent = u64::from(percent);
    let hundreds = (mib / 100).saturating_mul(percent);
    // Below 100 * u32::MAX, so it cannot overflow.
    let rest = (mib % 100) * percent / 100;
    hundreds.saturating_add(rest)
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

    fn settings(reserve_mib: u32, overcommit_percent: u32) -> HostMemorySettings {
        HostMemorySettings {
            reserve_mib,
            overcommit_percent,
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
        settings: HostMemorySettings,
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
        check(&requested, &running, &[], settings(4096, 100)).unwrap();
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
        let message = error(check(&requested, &running, &[], settings(16 * 1024, 100)));
        for expected in [
            "Not enough memory on the server",
            "node 'rel2604' needs 30720 MiB",
            "only 26624 MiB",
            "4096 MiB short",
            "The server has 163840 MiB",
            "16384 MiB host reserve",
            "overcommit of 100%",
            "guests may have 147456 MiB",
            "Running VMs and unikernels hold 120832 MiB (4)",
            "`host_memory.reserve_mib`",
            "`host_memory.overcommit_percent`",
            "sherpa.toml",
        ] {
            assert!(message.contains(expected), "{expected:?} not in: {message}");
        }
    }

    #[test]
    fn nodes_other_operations_are_starting_count_too() {
        let starting = [claim("other-lab", 100, None)];
        let requested = [claim("a", 40, None), claim("b", 20, None)];
        let message = error(check(&requested, &[], &starting, settings(4096, 100)));
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
    fn the_overcommit_percent_scales_what_guests_may_have() {
        let running = [claim("big", 150, None)];
        let requested = [claim("more", 60, None)];
        assert!(check(&requested, &running, &[], settings(4096, 100)).is_err());
        check(&requested, &running, &[], settings(4096, 150)).unwrap();
    }

    #[test]
    fn an_overcommit_of_100_percent_is_exact() {
        for mib in [0, 1, 99, 101, 12_345, 163_840, u64::MAX] {
            assert_eq!(scale_mib(mib, 100), mib);
        }
    }

    #[test]
    fn an_overcommit_of_150_percent_is_one_and_a_half_times() {
        assert_eq!(scale_mib(1000, 150), 1500);
        assert_eq!(scale_mib(147_456, 150), 221_184);
    }

    #[test]
    fn scaling_rounds_down_exactly() {
        // 110% of 12345 MiB is 13579.5 MiB, and of 1001 MiB 1101.1 MiB.
        assert_eq!(scale_mib(12_345, 110), 13_579);
        assert_eq!(scale_mib(1001, 110), 1101);
        assert_eq!(scale_mib(1, 1), 0);
        assert_eq!(scale_mib(199, 1), 1);
        // Odd values just below where a float would round up.
        assert_eq!(scale_mib(u64::MAX / 2, 100), u64::MAX / 2);
        assert_eq!(scale_mib((1 << 53) + 1, 100), (1 << 53) + 1);
    }

    #[test]
    fn huge_values_saturate_instead_of_overflowing() {
        assert_eq!(scale_mib(u64::MAX, 150), u64::MAX);
        assert_eq!(scale_mib(u64::MAX, u32::MAX), u64::MAX);
        assert_eq!(scale_mib(u64::MAX / 100, u32::MAX), u64::MAX);
        assert_eq!(scale_mib(u64::MAX / 2, 201), u64::MAX);
        // Just below the limit, the result is still exact: (2^63 - 1) * 1.99, rounded down.
        assert_eq!(scale_mib(u64::MAX / 2, 199), 18_354_510_353_341_003_855);
        let mut host = host();
        host.total_kib = u64::MAX;
        let requested = [claim("a", 1, None)];
        let committed = CommittedMemory {
            running: &[],
            starting: &[],
        };
        check_memory_fits(&requested, committed, &host, settings(0, u32::MAX)).unwrap();
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
            settings(0, 100),
        ));
        assert!(message.contains("less 40960 MiB in hugepages"), "{message}");
    }

    #[test]
    fn the_numa_node_oom_of_514_is_refused_naming_the_node_and_its_holders() {
        // The whole server has room, but NUMA node 0 does not.
        let running = [claim("agent", 60, Some(0)), claim("dev", 10, Some(1))];
        let requested = [claim("pinned", 24, Some(0))];
        let message = error(check(&requested, &running, &[], settings(4096, 100)));
        for expected in [
            "Not enough memory on NUMA node 0",
            "`numa_node = 0`",
            "node 'pinned' needs 24576 MiB",
            "NUMA node 0 has 81920 MiB",
            "2048 MiB share of the host reserve",
            "VMs held strictly on NUMA node 0 hold 61440 MiB: agent (61440 MiB)",
            r#"`numa_memory_mode = "preferred"`"#,
            "`host_memory.reserve_mib`",
        ] {
            assert!(message.contains(expected), "{expected:?} not in: {message}");
        }
    }

    #[test]
    fn unbound_memory_only_counts_against_the_whole_server() {
        let running = [claim("unpinned", 70, None)];
        let requested = [claim("pinned", 70, Some(0))];
        check(&requested, &running, &[], settings(4096, 100)).unwrap();
    }

    #[test]
    fn several_requested_nodes_on_one_numa_node_add_up() {
        let requested = [claim("a", 40, Some(1)), claim("b", 40, Some(1))];
        let message = error(check(&requested, &[], &[], settings(4096, 100)));
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
        let message = error(check(&requested, &[], &[], settings(4096, 100)));
        assert!(message.contains("NUMA node 7"), "{message}");
    }

    #[test]
    fn nothing_requested_always_fits() {
        let running = [claim("huge", 500, None)];
        check(&[], &running, &[], settings(4096, 100)).unwrap();
    }

    #[test]
    fn the_overcommit_percent_must_be_at_least_1() {
        check_host_memory_settings(settings(4096, 100)).unwrap();
        check_host_memory_settings(settings(0, 1)).unwrap();
        check_host_memory_settings(settings(0, u32::MAX)).unwrap();
        let message = error(check_host_memory_settings(settings(4096, 0)));
        assert!(
            message.contains("`host_memory.overcommit_percent` must be at least 1, not 0"),
            "{message}"
        );
    }
}
