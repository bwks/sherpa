//! Server-wide admission of the nodes an operation will start.
//!
//! Before `up`, `redeploy` or a resume starts nodes, it asks for admission. Requests are
//! handled one at a time, in the order they arrive: the queue is a fair (FIFO) Tokio mutex.
//! Under it, the nodes' CPU pins are checked against the stored nodes and the nodes other
//! operations were admitted with, and their memory against the server's (see
//! `validate::check_memory_fits`). Memory held is that of libvirt's running domains and of
//! Docker's running containers with a memory limit; a container without one counts as nothing.
//! Containers take no part in CPU pinning: their CPU quota holds no particular host CPUs, and
//! only has to be no more than the server's CPUs. Admitted nodes go into a pending ledger, so the next request
//! counts them before libvirt shows them running; the ledger keeps them until the operation
//! drops its [`Admission`]. A domain that is both running and pending counts once, as pending.
//!
//! The ledger is in memory: a server restart ends the operations it held entries for.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use anyhow::{Context, Result};
use tokio::sync::Mutex as FifoMutex;
use tracing::instrument;
use virt::connect::Connect;

use container::{ContainerMemory, Docker};
use libvirt::DomainMemory;
use shared::data::{HostMemorySettings, NodeConfig, NodeCpuPins, NodeKind};

use super::node_resources::{
    PendingPins, check_cpu_pins_on_server, domain_name, planned_resources,
};
use crate::validate::{CommittedMemory, MemoryClaim, check_container_cpus, check_memory_fits};

/// The admission queue. Tokio's mutex grants the lock in the order it was asked for.
static QUEUE: OnceLock<FifoMutex<()>> = OnceLock::new();
/// Nodes admitted and not yet released.
static LEDGER: OnceLock<Mutex<Vec<Pending>>> = OnceLock::new();
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

/// A node an operation asks to start.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    /// The node's domain name, `<node>-<lab id>`.
    domain: String,
    /// Its memory; `None` for a container without a memory limit.
    memory: Option<MemoryClaim>,
    /// Its CPU pins, when it pins any.
    pins: Option<NodeCpuPins>,
}

/// An admitted node in the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    ticket: u64,
    request: Request,
}

/// The nodes one operation was admitted to start. Dropping it releases them from the ledger;
/// by then they run, and libvirt counts them, or the operation has failed.
#[must_use = "dropping the admission releases the nodes it holds"]
#[derive(Debug)]
pub struct Admission {
    ticket: u64,
}

impl Drop for Admission {
    fn drop(&mut self) {
        ledger()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|pending| pending.ticket != self.ticket);
    }
}

fn queue() -> &'static FifoMutex<()> {
    QUEUE.get_or_init(|| FifoMutex::new(()))
}

fn ledger() -> &'static Mutex<Vec<Pending>> {
    LEDGER.get_or_init(|| Mutex::new(Vec::new()))
}

/// Admit `nodes` of lab `lab_id`, each with its image, to be created: by `up`, or by a
/// redeploy, which replaces a node of the same name. Their CPU pins must fit beside every other
/// node's, a container's CPU quota in the server's CPUs, and their memory in the server's.
///
/// # Errors
///
/// Fails when the pins overlap or name host CPUs or NUMA nodes the server does not have, when
/// a container's CPU quota is more than the server's CPUs, when the memory does not fit, or
/// when the database, libvirt or Docker cannot be read.
#[instrument(skip(db, qemu_conn, docker, settings, nodes), fields(%lab_id))]
pub async fn admit_lab_nodes(
    db: &db::Database,
    qemu_conn: &Connect,
    docker: &Docker,
    settings: HostMemorySettings,
    lab_id: &str,
    nodes: &[(&topology::NodeExpanded, &NodeConfig)],
) -> Result<Admission> {
    let requests = lab_requests(lab_id, nodes);
    let _queue = queue().lock().await;
    let pending = pending_entries();
    let pending_pins: Vec<PendingPins> = pending
        .iter()
        .filter_map(|entry| {
            entry.request.pins.clone().map(|pins| PendingPins {
                domain: entry.request.domain.clone(),
                pins,
            })
        })
        .collect();
    let expanded: Vec<&topology::NodeExpanded> = nodes.iter().map(|(node, _)| *node).collect();
    check_cpu_pins_on_server(db, qemu_conn, lab_id, &expanded, &pending_pins)
        .await
        .context("CPU pinning validation failed")?;
    check_container_quotas(qemu_conn, nodes)?;
    if requests.iter().any(|request| request.memory.is_some()) {
        let guests = guest_memory(qemu_conn, docker).await?;
        check_memory(qemu_conn, settings, &requests, &pending, &guests)?;
    }
    Ok(record(requests))
}

/// Admit the stopped nodes among `node_names` of lab `lab_id` to be started again: VMs,
/// unikernels and containers with a memory limit. Running and paused ones already hold their
/// memory, and their pins stay stored while they are stopped, so only the memory of stopped
/// ones is checked.
///
/// # Errors
///
/// Fails when their memory does not fit in the server's, or libvirt or Docker cannot be read.
#[instrument(skip(qemu_conn, docker, settings, node_names), fields(%lab_id))]
pub async fn admit_stopped_nodes(
    qemu_conn: &Connect,
    docker: &Docker,
    settings: HostMemorySettings,
    lab_id: &str,
    node_names: &[&str],
) -> Result<Admission> {
    let _queue = queue().lock().await;
    let pending = pending_entries();
    let guests = guest_memory(qemu_conn, docker).await?;
    let requests = stopped_requests(lab_id, node_names, &guests);
    if !requests.is_empty() {
        check_memory(qemu_conn, settings, &requests, &pending, &guests)?;
    }
    Ok(record(requests))
}

/// The memory of every guest on the server, running or not: libvirt's domains, and Docker's
/// containers with a memory limit.
async fn guest_memory(qemu_conn: &Connect, docker: &Docker) -> Result<Vec<DomainMemory>> {
    let mut guests = libvirt::list_domain_memory(qemu_conn)?;
    let containers = container::list_container_memory(docker)
        .await
        .context("Cannot read the memory limits of the server's containers")?;
    guests.extend(container_guests(containers));
    Ok(guests)
}

/// Containers with a memory limit as guests holding that memory, rounded up to whole KiB. A
/// container's memory is not bound to a NUMA node.
fn container_guests(containers: Vec<ContainerMemory>) -> Vec<DomainMemory> {
    containers
        .into_iter()
        .map(|container| DomainMemory {
            name: container.name,
            active: container.active,
            memory_kib: container.limit_bytes.div_ceil(1024),
            strict_numa_node: None,
        })
        .collect()
}

/// Check that each container's CPU quota is no more than the server's CPUs.
fn check_container_quotas(
    qemu_conn: &Connect,
    nodes: &[(&topology::NodeExpanded, &NodeConfig)],
) -> Result<()> {
    let quotas: Vec<(&str, u8)> = nodes
        .iter()
        .filter(|(_, image)| image.kind == NodeKind::Container)
        .filter_map(|(node, image)| {
            planned_resources(node, image)
                .cpu_count
                .map(|count| (node.name.as_str(), count))
        })
        .collect();
    if quotas.is_empty() {
        return Ok(());
    }
    let host = libvirt::host_cpus(qemu_conn)?;
    for (node_name, cpu_count) in quotas {
        check_container_cpus(node_name, cpu_count, host.cpus)?;
    }
    Ok(())
}

/// Check the memory of `requests` against the server's and what is committed.
fn check_memory(
    qemu_conn: &Connect,
    settings: HostMemorySettings,
    requests: &[Request],
    pending: &[Pending],
    domains: &[DomainMemory],
) -> Result<()> {
    let host = libvirt::host_memory(qemu_conn)?;
    let claims: Vec<MemoryClaim> = requests
        .iter()
        .filter_map(|request| request.memory.clone())
        .collect();
    let requested: HashSet<&str> = requests
        .iter()
        .map(|request| request.domain.as_str())
        .collect();
    let (running, starting) = committed_memory(domains, pending, &requested);
    check_memory_fits(
        &claims,
        CommittedMemory {
            running: &running,
            starting: &starting,
        },
        &host,
        settings,
    )
    .context("Memory admission failed")
}

/// What each node asks for: VMs, unikernels and containers with a limit their memory, VMs held strictly to a NUMA node
/// that node, and every tuned node its CPU pins. A VM that only prefers its NUMA node claims
/// memory on the whole server, since the kernel may place it on any node.
fn lab_requests(lab_id: &str, nodes: &[(&topology::NodeExpanded, &NodeConfig)]) -> Vec<Request> {
    nodes
        .iter()
        .map(|(node, image)| {
            let numa_node = if image.kind == NodeKind::VirtualMachine {
                node.numa.strict_node().map(u32::from)
            } else {
                None
            };
            let pins = NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk);
            let memory = planned_resources(node, image)
                .memory
                .map(|memory| MemoryClaim {
                    name: node.name.clone(),
                    memory_mib: u64::from(memory),
                    numa_node,
                });
            Request {
                domain: domain_name(&node.name, lab_id),
                memory,
                pins: Some(pins).filter(|pins| !pins.is_empty()),
            }
        })
        .collect()
}

/// The stopped guests among `node_names`, with the memory libvirt or Docker has for them.
fn stopped_requests(lab_id: &str, node_names: &[&str], domains: &[DomainMemory]) -> Vec<Request> {
    node_names
        .iter()
        .filter_map(|node_name| {
            let domain = domain_name(node_name, lab_id);
            let found = domains
                .iter()
                .find(|entry| entry.name == domain && !entry.active)?;
            Some(Request {
                memory: Some(MemoryClaim {
                    name: (*node_name).to_string(),
                    memory_mib: found.memory_kib.div_ceil(1024),
                    numa_node: found.strict_numa_node,
                }),
                domain,
                pins: None,
            })
        })
        .collect()
}

/// The memory held by running guests, domains and limited containers, and by nodes pending in
/// other operations, leaving out the `requested` ones, whose new memory is checked. A guest that
/// is running and pending
/// counts once, as pending, since the operation holding it may be replacing it.
fn committed_memory(
    domains: &[DomainMemory],
    pending: &[Pending],
    requested: &HashSet<&str>,
) -> (Vec<MemoryClaim>, Vec<MemoryClaim>) {
    let pending_domains: HashSet<&str> = pending
        .iter()
        .filter(|entry| entry.request.memory.is_some())
        .map(|entry| entry.request.domain.as_str())
        .collect();
    let running = domains
        .iter()
        .filter(|domain| domain.active)
        .filter(|domain| {
            let name = domain.name.as_str();
            !requested.contains(name) && !pending_domains.contains(name)
        })
        .map(|domain| MemoryClaim {
            name: domain.name.clone(),
            memory_mib: domain.memory_kib.div_ceil(1024),
            numa_node: domain.strict_numa_node,
        })
        .collect();
    let starting = pending
        .iter()
        .filter(|entry| !requested.contains(entry.request.domain.as_str()))
        .filter_map(|entry| {
            entry.request.memory.clone().map(|claim| MemoryClaim {
                name: entry.request.domain.clone(),
                ..claim
            })
        })
        .collect();
    (running, starting)
}

/// The nodes admitted and not yet released.
fn pending_entries() -> Vec<Pending> {
    ledger()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Put `requests` in the ledger under a new ticket.
fn record(requests: Vec<Request>) -> Admission {
    let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
    ledger()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend(
            requests
                .into_iter()
                .map(|request| Pending { ticket, request }),
        );
    Admission { ticket }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::services::error::ServiceError;
    use container::{
        ContainerSpec, docker_connection, kill_container, remove_container, run_container,
    };
    use libvirt::Qemu;
    use shared::data::{ContainerNetworkAttachment, NodeCpu, NodeModel, NodeNuma, NumaMemoryMode};

    fn running(name: &str, mib: u64, numa_node: Option<u32>) -> DomainMemory {
        DomainMemory {
            name: name.to_string(),
            active: true,
            memory_kib: mib * 1024,
            strict_numa_node: numa_node,
        }
    }

    fn request(domain: &str, mib: Option<u64>) -> Request {
        Request {
            domain: domain.to_string(),
            memory: mib.map(|memory_mib| MemoryClaim {
                name: "node".to_string(),
                memory_mib,
                numa_node: None,
            }),
            pins: None,
        }
    }

    fn pending(ticket: u64, domain: &str, mib: Option<u64>) -> Pending {
        Pending {
            ticket,
            request: request(domain, mib),
        }
    }

    fn names(claims: &[MemoryClaim]) -> Vec<&str> {
        claims.iter().map(|claim| claim.name.as_str()).collect()
    }

    #[test]
    fn running_domains_count_unless_requested_or_pending() {
        let domains = [
            running("a-lab1", 1024, Some(0)),
            running("b-lab1", 2048, None),
            running("c-lab2", 4096, None),
            DomainMemory {
                active: false,
                ..running("stopped-lab1", 8192, None)
            },
        ];
        let pending = [
            pending(1, "b-lab1", Some(3000)),
            pending(1, "d-lab3", Some(10)),
        ];
        let requested = HashSet::from(["c-lab2"]);
        let (running, starting) = committed_memory(&domains, &pending, &requested);
        assert_eq!(names(&running), ["a-lab1"]);
        assert_eq!(running[0].memory_mib, 1024);
        assert_eq!(running[0].numa_node, Some(0));
        assert_eq!(names(&starting), ["b-lab1", "d-lab3"]);
        assert_eq!(starting[0].memory_mib, 3000);
    }

    #[test]
    fn a_pending_container_does_not_hide_a_running_domain() {
        let domains = [running("a-lab1", 1024, None)];
        let pending = [pending(1, "a-lab1", None)];
        let (running, starting) = committed_memory(&domains, &pending, &HashSet::new());
        assert_eq!(names(&running), ["a-lab1"]);
        assert_eq!(starting, []);
    }

    #[test]
    fn a_requested_domain_pending_elsewhere_is_left_out() {
        let pending = [pending(1, "a-lab1", Some(1024))];
        let (_, starting) = committed_memory(&[], &pending, &HashSet::from(["a-lab1"]));
        assert_eq!(starting, []);
    }

    #[test]
    fn running_memory_is_rounded_up_to_whole_mib() {
        let domains = [DomainMemory {
            memory_kib: 1025,
            ..running("a-lab1", 0, None)
        }];
        let (running, _) = committed_memory(&domains, &[], &HashSet::new());
        assert_eq!(running[0].memory_mib, 2);
    }

    #[test]
    fn lab_requests_take_memory_numa_node_and_pins_by_kind() {
        let vm_image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let unikernel_image = NodeConfig::get_model(NodeModel::NanosUnikernel);
        let container_image = NodeConfig::get_model(NodeModel::MongoDb);
        let cpu = NodeCpu {
            pinning: Some(vec![2, 3]),
            ..NodeCpu::default()
        };
        let numa = NodeNuma {
            node: Some(1),
            memory_mode: None,
        };
        let vm = topology::NodeExpanded {
            name: "vm".to_string(),
            memory: Some(8192),
            cpu: cpu.clone(),
            numa: numa.clone(),
            ..Default::default()
        };
        let unikernel = topology::NodeExpanded {
            name: "uk".to_string(),
            cpu,
            numa,
            ..Default::default()
        };
        let container = topology::NodeExpanded {
            name: "db".to_string(),
            memory: Some(4096),
            ..Default::default()
        };
        let unlimited = topology::NodeExpanded {
            name: "cache".to_string(),
            ..Default::default()
        };
        let requests = lab_requests(
            "lab1",
            &[
                (&vm, &vm_image),
                (&unikernel, &unikernel_image),
                (&container, &container_image),
                (&unlimited, &container_image),
            ],
        );
        assert_eq!(requests[0].domain, "vm-lab1");
        assert_eq!(
            requests[0].memory,
            Some(MemoryClaim {
                name: "vm".to_string(),
                memory_mib: 8192,
                numa_node: Some(1),
            })
        );
        assert_eq!(
            requests[0].pins.as_ref().map(|pins| pins.vcpu_cpus.clone()),
            Some(vec![2, 3])
        );
        let unikernel_memory = requests[1].memory.as_ref().unwrap();
        assert_eq!(
            unikernel_memory.memory_mib,
            u64::from(unikernel_image.memory)
        );
        assert_eq!(unikernel_memory.numa_node, None);
        assert_eq!(
            requests[2].memory,
            Some(MemoryClaim {
                name: "db".to_string(),
                memory_mib: 4096,
                numa_node: None,
            })
        );
        assert_eq!(requests[2].pins, None);
        // A container without a limit claims nothing.
        assert_eq!(requests[3].memory, None);
    }

    #[test]
    fn limited_containers_count_as_guests_by_their_limit() {
        let guests = container_guests(vec![
            ContainerMemory {
                name: "db-lab1".to_string(),
                active: true,
                limit_bytes: 1024 * 1024 * 1024,
            },
            ContainerMemory {
                name: "cache-lab1".to_string(),
                active: false,
                limit_bytes: 1025,
            },
        ]);
        assert_eq!(
            guests,
            [
                running("db-lab1", 1024, None),
                DomainMemory {
                    active: false,
                    memory_kib: 2,
                    ..running("cache-lab1", 0, None)
                },
            ]
        );
        let (running, _) = committed_memory(&guests, &[], &HashSet::new());
        assert_eq!(names(&running), ["db-lab1"]);
        let requests = stopped_requests("lab1", &["cache"], &guests);
        assert_eq!(requests[0].domain, "cache-lab1");
    }

    #[test]
    fn a_vm_preferring_a_numa_node_claims_memory_on_the_server_only() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let vm = |numa_memory_mode| topology::NodeExpanded {
            name: "vm".to_string(),
            memory: Some(8192),
            numa: NodeNuma {
                node: Some(1),
                memory_mode: numa_memory_mode,
            },
            ..Default::default()
        };
        let preferred = vm(Some(NumaMemoryMode::Preferred));
        let strict = vm(Some(NumaMemoryMode::Strict));
        let requests = lab_requests("lab1", &[(&preferred, &image)]);
        assert_eq!(
            requests[0].memory,
            Some(MemoryClaim {
                name: "vm".to_string(),
                memory_mib: 8192,
                numa_node: None,
            })
        );
        let requests = lab_requests("lab1", &[(&strict, &image)]);
        assert_eq!(
            requests[0]
                .memory
                .as_ref()
                .and_then(|claim| claim.numa_node),
            Some(1)
        );
    }

    #[test]
    fn only_stopped_domains_are_requested_on_resume() {
        let domains = [
            DomainMemory {
                active: false,
                ..running("a-lab1", 2048, Some(0))
            },
            running("b-lab1", 1024, None),
        ];
        let requests = stopped_requests("lab1", &["a", "b", "missing"], &domains);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].domain, "a-lab1");
        assert_eq!(
            requests[0].memory,
            Some(MemoryClaim {
                name: "a".to_string(),
                memory_mib: 2048,
                numa_node: Some(0),
            })
        );
    }

    #[test]
    fn dropping_an_admission_releases_only_its_nodes() {
        let first = record(vec![request("ledger-test-a", Some(1))]);
        let second = record(vec![request("ledger-test-b", Some(1))]);
        let domains = |entries: Vec<Pending>| -> Vec<String> {
            entries
                .into_iter()
                .map(|entry| entry.request.domain)
                .filter(|domain| domain.starts_with("ledger-test-"))
                .collect()
        };
        assert_eq!(
            domains(pending_entries()),
            ["ledger-test-a", "ledger-test-b"]
        );
        drop(first);
        assert_eq!(domains(pending_entries()), ["ledger-test-b"]);
        drop(second);
        assert_eq!(domains(pending_entries()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn the_queue_admits_one_request_at_a_time_in_arrival_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let held = queue().lock().await;
        let mut waiters = Vec::new();
        for index in 0..3 {
            let order = order.clone();
            waiters.push(tokio::spawn(async move {
                let _queue = queue().lock().await;
                order.lock().unwrap().push(index);
            }));
            // Let each waiter reach the queue before the next one.
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(held);
        for waiter in waiters {
            waiter.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
    }

    /// The smallest node the test below can use, in MiB.
    const MIN_TEST_NODE_MIB: u64 = 256;

    /// The memory in MiB guests may still be given on the server: what the admission allows
    /// them, less what running guests hold. Read the way the admission reads it.
    async fn guest_memory_left(
        conn: &Connect,
        docker: &Docker,
        settings: HostMemorySettings,
    ) -> u64 {
        let host = libvirt::host_memory(conn).unwrap();
        let allowed = (host.total_kib / 1024)
            .saturating_sub(host.hugepages_kib.div_ceil(1024))
            .saturating_sub(u64::from(settings.reserve_mib));
        let percent = u64::from(settings.overcommit_percent);
        let limit = allowed / 100 * percent + allowed % 100 * percent / 100;
        let held: u64 = guest_memory(conn, docker)
            .await
            .unwrap()
            .iter()
            .filter(|domain| domain.active)
            .map(|domain| domain.memory_kib.div_ceil(1024))
            .sum();
        limit.saturating_sub(held)
    }

    /// Against the real libvirt: two nodes that each fit, but not together, are admitted one
    /// at a time, and the second only once the first is released. Each node is sized from
    /// the server's free guest memory, just over half of it, so this works on any server; it
    /// starts nothing. It skips, saying so, on a server with too little memory left.
    #[tokio::test]
    #[ignore = "requires a libvirt daemon and a Docker daemon"]
    async fn a_pending_node_holds_its_memory_until_released() {
        let db = db::open_in_memory().await.unwrap();
        let conn = Qemu::default().connect().unwrap();
        let docker = docker_connection().unwrap();
        let settings = HostMemorySettings::default();
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let left = guest_memory_left(&conn, &docker, settings).await;
        let size = left / 2 + 1;
        let Some(memory) = u16::try_from(size)
            .ok()
            .filter(|_| size >= MIN_TEST_NODE_MIB)
        else {
            eprintln!(
                "SKIPPED a_pending_node_holds_its_memory_until_released: {left} MiB of guest \
                 memory is left on this server, but the test needs between {} and {} MiB to \
                 size two nodes that fit singly but not together (a node's memory is a u16)",
                MIN_TEST_NODE_MIB * 2,
                u64::from(u16::MAX) * 2
            );
            return;
        };
        let node = |name: &str| topology::NodeExpanded {
            name: name.to_string(),
            memory: Some(memory),
            ..Default::default()
        };
        let (first, second) = (node("first"), node("second"));

        let held = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "admtest1",
            &[(&first, &image)],
        )
        .await
        .unwrap();
        let refused = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "admtest2",
            &[(&second, &image)],
        )
        .await
        .unwrap_err();
        let message = format!("{refused:#}");
        assert!(
            matches!(
                ServiceError::find(&refused),
                Some(ServiceError::InsufficientMemory { shortfall, .. })
                    if shortfall.requested_mib == size && shortfall.numa_node.is_none()
            ),
            "{message}"
        );
        assert!(
            message.contains(&format!("node 'second' needs {size} MiB")),
            "{message}"
        );
        assert!(
            message.contains(&format!(
                "nodes other operations are starting hold {size} MiB (1)"
            )),
            "{message}"
        );

        drop(held);
        let admitted = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "admtest2",
            &[(&second, &image)],
        )
        .await
        .unwrap();
        drop(admitted);
    }

    /// Against the real libvirt: two labs whose nodes pin the same host CPU are admitted one at
    /// a time. While the first lab's admission is held, before any node of it is stored or
    /// running, the second is refused; once it is released, the second is admitted. Checking
    /// the pins against stored nodes only would let two `up`s both pass.
    #[tokio::test]
    #[ignore = "requires a libvirt daemon and a Docker daemon"]
    async fn a_pending_node_holds_its_cpu_pins_until_released() {
        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        let conn = Qemu::default().connect().unwrap();
        let docker = docker_connection().unwrap();
        let settings = HostMemorySettings::default();
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let node = |name: &str| topology::NodeExpanded {
            name: name.to_string(),
            memory: Some(256),
            cpu: NodeCpu {
                pinning: Some(vec![0]),
                ..NodeCpu::default()
            },
            ..Default::default()
        };
        let (first, second) = (node("first"), node("second"));

        let held = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "pintest1",
            &[(&first, &image)],
        )
        .await
        .unwrap();
        let refused = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "pintest2",
            &[(&second, &image)],
        )
        .await
        .unwrap_err();
        let message = format!("{refused:#}");
        assert!(
            message.contains("CPU pinning validation failed"),
            "{message}"
        );
        assert!(
            message.contains("being created by another operation"),
            "{message}"
        );

        drop(held);
        let admitted = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "pintest2",
            &[(&second, &image)],
        )
        .await
        .unwrap();
        drop(admitted);
    }

    /// Against the real libvirt and Docker: a lab of containers whose memory limits together
    /// are more than the server's guest memory left is refused with the memory refusal. It
    /// starts nothing.
    #[tokio::test]
    #[ignore = "requires a libvirt daemon and a Docker daemon"]
    async fn a_lab_of_containers_that_does_not_fit_is_refused_with_the_memory_error() {
        let db = db::open_in_memory().await.unwrap();
        let conn = Qemu::default().connect().unwrap();
        let docker = docker_connection().unwrap();
        let settings = HostMemorySettings::default();
        let image = NodeConfig::get_model(NodeModel::GenericContainer);
        let left = guest_memory_left(&conn, &docker, settings).await;
        let count = left / u64::from(u16::MAX) + 1;
        let nodes: Vec<topology::NodeExpanded> = (0..count)
            .map(|index| topology::NodeExpanded {
                name: format!("ctr{index}"),
                memory: Some(u16::MAX),
                ..Default::default()
            })
            .collect();
        let admitted: Vec<(&topology::NodeExpanded, &NodeConfig)> =
            nodes.iter().map(|node| (node, &image)).collect();
        let refused = admit_lab_nodes(&db, &conn, &docker, settings, "ctrtest1", &admitted)
            .await
            .unwrap_err();
        let message = format!("{refused:#}");
        assert!(
            matches!(
                ServiceError::find(&refused),
                Some(ServiceError::InsufficientMemory { shortfall, .. })
                    if shortfall.requested_mib == count * u64::from(u16::MAX)
            ),
            "{message}"
        );
    }

    /// Against the real libvirt and Docker: a running container with a memory limit holds its
    /// memory, so a node that fits only without it is refused, and admitted once the
    /// container is gone. It skips, saying so, on a server with too little memory left.
    #[tokio::test]
    #[ignore = "requires a libvirt daemon and a Docker daemon"]
    async fn a_running_container_holds_its_memory_limit() {
        let db = db::open_in_memory().await.unwrap();
        let conn = Qemu::default().connect().unwrap();
        let docker = docker_connection().unwrap();
        let settings = HostMemorySettings::default();
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let name = "admtest-ctr-limit";
        let _ = kill_container(&docker, name).await;
        let _ = remove_container(&docker, name).await;
        let left = guest_memory_left(&conn, &docker, settings).await;
        let size = left / 2 + 1;
        let Some(memory) = u16::try_from(size)
            .ok()
            .filter(|_| size >= MIN_TEST_NODE_MIB)
        else {
            eprintln!(
                "SKIPPED a_running_container_holds_its_memory_limit: {left} MiB of guest memory \
                 is left on this server, but the test needs between {} and {} MiB",
                MIN_TEST_NODE_MIB * 2,
                u64::from(u16::MAX) * 2
            );
            return;
        };
        run_container(
            &docker,
            ContainerSpec {
                name: name.to_string(),
                image: "alpine:latest".to_string(),
                env_vars: vec![],
                volumes: vec![],
                capabilities: vec![],
                management_network: ContainerNetworkAttachment {
                    name: "bridge".to_string(),
                    ipv4_address: None,
                    ipv6_address: None,
                    linux_interface_name: None,
                    admin_down: false,
                },
                additional_networks: vec![],
                commands: vec!["sleep".to_string(), "3600".to_string()],
                privileged: false,
                shm_size: None,
                user: None,
                memory_mib: Some(size),
                cpu_count: None,
            },
        )
        .await
        .unwrap();
        let node = topology::NodeExpanded {
            name: "vm".to_string(),
            memory: Some(memory),
            ..Default::default()
        };
        let refused = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "ctrtest2",
            &[(&node, &image)],
        )
        .await;
        let _ = kill_container(&docker, name).await;
        let _ = remove_container(&docker, name).await;
        let refused = refused.unwrap_err();
        let message = format!("{refused:#}");
        assert!(
            matches!(
                ServiceError::find(&refused),
                Some(ServiceError::InsufficientMemory { .. })
            ),
            "{message}"
        );
        let admitted = admit_lab_nodes(
            &db,
            &conn,
            &docker,
            settings,
            "ctrtest2",
            &[(&node, &image)],
        )
        .await
        .unwrap();
        drop(admitted);
    }
}
