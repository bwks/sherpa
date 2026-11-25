use std::fmt;

use anyhow::{Result, bail};

use libvirt::HostMemory;
use shared::data::HostMemorySettings;
use shared::error::MemoryShortfall;

use crate::services::error::ServiceError;

/// Memory one VM or unikernel takes on the server, or will take once started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryClaim {
    pub name: String,
    pub memory_mib: u64,
    pub numa_node: Option<u32>,
}

/// Why requested nodes do not fit in the server's memory, with every number the check used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryShortage {
    Server {
        requested: Vec<String>,
        needed_mib: u64,
        left_mib: u64,
        total_mib: u64,
        hugepages_mib: u64,
        reserve_mib: u64,
        overcommit_percent: u32,
        limit_mib: u64,
        running_mib: u64,
        running_count: usize,
        starting_mib: u64,
        starting_count: usize,
    },
    NumaNode {
        numa_node: u32,
        requested: Vec<String>,
        needed_mib: u64,
        left_mib: u64,
        node_mib: u64,
        hugepages_mib: u64,
        reserve_share_mib: u64,
        overcommit_percent: u32,
        limit_mib: u64,
        held_mib: u64,
        holders: Vec<MemoryClaim>,
    },
}

/// Memory already promised to other guests.
#[derive(Clone, Copy, Debug)]
pub struct CommittedMemory<'a> {
    pub running: &'a [MemoryClaim],
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
/// Fails with [`ServiceError::InsufficientMemory`] when the nodes do not fit, naming the
/// numbers that fell short and the settings that change them, or when a requested NUMA node
/// does not exist.
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
        bail!(ServiceError::InsufficientMemory {
            shortage: MemoryShortage::Server {
                requested: names(requested),
                needed_mib: needed,
                left_mib: left,
                total_mib,
                hugepages_mib,
                reserve_mib,
                overcommit_percent: percent,
                limit_mib: limit,
                running_mib: running,
                running_count: committed.running.len(),
                starting_mib: starting,
                starting_count: committed.starting.len(),
            },
        });
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
        bail!(ServiceError::NumaNodeNotOnServer { numa_node });
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
        bail!(ServiceError::InsufficientMemory {
            shortage: MemoryShortage::NumaNode {
                numa_node,
                requested: names(&wanted),
                needed_mib: needed,
                left_mib: left,
                node_mib: cell_mib,
                hugepages_mib,
                reserve_share_mib: reserve_share,
                overcommit_percent: percent,
                limit_mib: limit,
                held_mib: held,
                holders,
            },
        });
    }
    Ok(())
}

/// Check the `[host_memory]` settings.
///
/// # Errors
///
/// `ServiceError::OvercommitPercentTooLow` when the overcommit percentage is 0.
pub fn check_host_memory_settings(settings: HostMemorySettings) -> Result<()> {
    let percent = settings.overcommit_percent;
    if percent < 1 {
        bail!(ServiceError::OvercommitPercentTooLow { percent });
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

/// The names of the claims' nodes.
fn names(claims: &[MemoryClaim]) -> Vec<String> {
    claims.iter().map(|claim| claim.name.clone()).collect()
}

/// "node 'a' needs" or "nodes 'a', 'b' need".
fn needing(names: &[String]) -> String {
    let quoted = names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() == 1 {
        format!("node {quoted} needs")
    } else {
        format!("nodes {quoted} need")
    }
}

impl MemoryShortage {
    /// The shortfall a client is sent: the memory needed, the memory left, and the NUMA node
    /// that is short.
    #[must_use]
    pub fn shortfall(&self) -> MemoryShortfall {
        match self {
            Self::Server {
                needed_mib,
                left_mib,
                ..
            } => MemoryShortfall {
                requested_mib: *needed_mib,
                available_mib: *left_mib,
                numa_node: None,
            },
            Self::NumaNode {
                numa_node,
                needed_mib,
                left_mib,
                ..
            } => MemoryShortfall {
                requested_mib: *needed_mib,
                available_mib: *left_mib,
                numa_node: Some(*numa_node),
            },
        }
    }
}

impl fmt::Display for MemoryShortage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Server {
                requested,
                needed_mib,
                left_mib,
                total_mib,
                hugepages_mib,
                reserve_mib,
                overcommit_percent,
                limit_mib,
                running_mib,
                running_count,
                starting_mib,
                starting_count,
            } => write!(
                f,
                "Not enough memory on the server: {} {needed_mib} MiB, but only {left_mib} MiB of \
                 guest memory is left, {} MiB short. The server has {total_mib} MiB; less \
                 {hugepages_mib} MiB in hugepages and the {reserve_mib} MiB host reserve, at an \
                 overcommit of {overcommit_percent}%, guests may have {limit_mib} MiB. Running \
                 VMs, unikernels and containers hold {running_mib} MiB ({running_count}), and \
                 nodes other operations are starting hold {starting_mib} MiB ({starting_count}). \
                 Stop or destroy nodes to free memory, or an admin can lower \
                 `host_memory.reserve_mib` or raise `host_memory.overcommit_percent` in \
                 sherpa.toml.",
                needing(requested),
                needed_mib - left_mib,
            ),
            Self::NumaNode {
                numa_node,
                requested,
                needed_mib,
                left_mib,
                node_mib,
                hugepages_mib,
                reserve_share_mib,
                overcommit_percent,
                limit_mib,
                held_mib,
                holders,
            } => {
                let holder_list = if holders.is_empty() {
                    "none".to_string()
                } else {
                    holders
                        .iter()
                        .map(|claim| format!("{} ({} MiB)", claim.name, claim.memory_mib))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                write!(
                    f,
                    r#"Not enough memory on NUMA node {numa_node} for nodes held there with `numa_node = {numa_node}`: {} {needed_mib} MiB, but only {left_mib} MiB is left, {} MiB short. NUMA node {numa_node} has {node_mib} MiB; less {hugepages_mib} MiB in hugepages and its {reserve_share_mib} MiB share of the host reserve, at an overcommit of {overcommit_percent}%, guests may have {limit_mib} MiB. VMs held strictly on NUMA node {numa_node} hold {held_mib} MiB: {holder_list}. Memory held on one NUMA node cannot use another node's. Choose another `numa_node`, set `numa_memory_mode = "preferred"` to let the node's memory spill to other NUMA nodes, stop or destroy those nodes, or an admin can lower `host_memory.reserve_mib` or raise `host_memory.overcommit_percent` in sherpa.toml."#,
                    needing(requested),
                    needed_mib - left_mib,
                )
            }
        }
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
    use shared::error::MemoryShortfall;

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

    fn error(result: Result<()>) -> ServiceError {
        result
            .unwrap_err()
            .downcast::<ServiceError>()
            .expect("a service error")
    }

    fn shortage(result: Result<()>) -> MemoryShortage {
        match error(result) {
            ServiceError::InsufficientMemory { shortage } => shortage,
            other => panic!("not a memory shortage: {other}"),
        }
    }

    fn shortfall(error: &anyhow::Error) -> Option<MemoryShortfall> {
        match ServiceError::find(error) {
            Some(ServiceError::InsufficientMemory { shortage }) => Some(shortage.shortfall()),
            _ => None,
        }
    }

    #[test]
    fn nodes_that_fit_beside_the_running_ones_are_admitted() {
        let running = [claim("dev", 48, Some(1)), claim("rel2404", 30, None)];
        let requested = [claim("rel2604", 30, None), claim("agent", 24, Some(0))];
        check(&requested, &running, &[], settings(4096, 100)).unwrap();
    }

    #[test]
    fn the_global_oom_of_514_is_refused_carrying_every_number() {
        // 148 GiB of guests on a 160 GiB server leaves less than the 16 GiB reserve.
        let running = [
            claim("dev", 48, Some(1)),
            claim("agent", 24, Some(0)),
            claim("rel2404", 30, None),
            claim("win", 16, None),
        ];
        let requested = [claim("rel2604", 30, None)];
        assert_eq!(
            shortage(check(&requested, &running, &[], settings(16 * 1024, 100))),
            MemoryShortage::Server {
                requested: vec!["rel2604".to_owned()],
                needed_mib: 30720,
                left_mib: 26624,
                total_mib: 163_840,
                hugepages_mib: 0,
                reserve_mib: 16384,
                overcommit_percent: 100,
                limit_mib: 147_456,
                running_mib: 120_832,
                running_count: 4,
                starting_mib: 0,
                starting_count: 0,
            }
        );
    }

    #[test]
    fn nodes_other_operations_are_starting_count_too() {
        let starting = [claim("other-lab", 100, None)];
        let requested = [claim("a", 40, None), claim("b", 20, None)];
        let MemoryShortage::Server {
            requested,
            needed_mib,
            starting_mib,
            starting_count,
            ..
        } = shortage(check(&requested, &[], &starting, settings(4096, 100)))
        else {
            panic!("not a shortage of the server's memory");
        };
        assert_eq!(requested, ["a", "b"]);
        assert_eq!(needed_mib, 61440);
        assert_eq!((starting_mib, starting_count), (102_400, 1));
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
        let MemoryShortage::Server { hugepages_mib, .. } = shortage(check_memory_fits(
            &requested,
            committed,
            &host,
            settings(0, 100),
        )) else {
            panic!("not a shortage of the server's memory");
        };
        assert_eq!(hugepages_mib, 40960);
    }

    #[test]
    fn the_numa_node_oom_of_514_is_refused_carrying_the_node_and_its_holders() {
        // The whole server has room, but NUMA node 0 does not.
        let running = [claim("agent", 60, Some(0)), claim("dev", 10, Some(1))];
        let requested = [claim("pinned", 24, Some(0))];
        assert_eq!(
            shortage(check(&requested, &running, &[], settings(4096, 100))),
            MemoryShortage::NumaNode {
                numa_node: 0,
                requested: vec!["pinned".to_owned()],
                needed_mib: 24576,
                left_mib: 18432,
                node_mib: 81920,
                hugepages_mib: 0,
                reserve_share_mib: 2048,
                overcommit_percent: 100,
                limit_mib: 79872,
                held_mib: 61440,
                holders: vec![claim("agent", 60, Some(0))],
            }
        );
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
        let MemoryShortage::NumaNode {
            numa_node,
            requested,
            needed_mib,
            ..
        } = shortage(check(&requested, &[], &[], settings(4096, 100)))
        else {
            panic!("not a shortage of a NUMA node's memory");
        };
        assert_eq!(numa_node, 1);
        assert_eq!(requested, ["a", "b"]);
        assert_eq!(needed_mib, 81920);
    }

    #[test]
    fn a_numa_node_the_server_does_not_have_is_refused() {
        let requested = [claim("a", 1, Some(7))];
        assert_eq!(
            error(check(&requested, &[], &[], settings(4096, 100))),
            ServiceError::NumaNodeNotOnServer { numa_node: 7 }
        );
    }

    #[test]
    fn a_server_shortfall_is_an_insufficient_memory_refusal() {
        let running = [claim("dev", 48, Some(1)), claim("rel2404", 70, None)];
        let requested = [claim("rel2604", 30, None)];
        let refused = check(&requested, &running, &[], settings(16 * 1024, 100)).unwrap_err();
        assert_eq!(
            shortfall(&refused),
            Some(MemoryShortfall {
                requested_mib: 30720,
                available_mib: 26624,
                numa_node: None,
            })
        );
    }

    #[test]
    fn a_numa_node_shortfall_names_the_node_in_the_refusal() {
        let running = [claim("agent", 60, Some(0)), claim("dev", 10, Some(1))];
        let requested = [claim("pinned", 24, Some(0))];
        let refused = check(&requested, &running, &[], settings(4096, 100)).unwrap_err();
        assert_eq!(
            shortfall(&refused),
            Some(MemoryShortfall {
                requested_mib: 24576,
                available_mib: 18432,
                numa_node: Some(0),
            })
        );
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
        assert_eq!(
            error(check_host_memory_settings(settings(4096, 0))),
            ServiceError::OvercommitPercentTooLow { percent: 0 }
        );
    }
}
