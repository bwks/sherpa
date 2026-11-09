//! Server-wide admission of the VMs and unikernels an operation will start.
//!
//! Before `up`, `redeploy` or a resume starts nodes, it asks for admission. Requests are
//! handled one at a time, in the order they arrive: the queue is a fair (FIFO) Tokio mutex.
//! Under it, the nodes' CPU pins are checked against the stored nodes and the nodes other
//! operations were admitted with, and their memory against the server's (see
//! `validate::check_memory_fits`). Admitted nodes go into a pending ledger, so the next request
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

use libvirt::DomainMemory;
use shared::data::{HostMemorySettings, NodeConfig, NodeCpuPins, NodeKind};

use super::node_resources::{
    PendingPins, check_cpu_pins_on_server, domain_name, planned_resources,
};
use crate::validate::{CommittedMemory, MemoryClaim, check_memory_fits};

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
    /// Its memory, for VMs and unikernels; `None` for containers, which Sherpa does not limit.
    memory: Option<MemoryClaim>,
    /// Its CPU pins, when it has performance tuning.
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
/// node's, and their memory in the server's.
///
/// # Errors
///
/// Fails when the pins overlap or name host CPUs or NUMA nodes the server does not have, when
/// the memory does not fit, or when the database or libvirt cannot be read.
#[instrument(skip(db, qemu_conn, settings, nodes), fields(%lab_id))]
pub async fn admit_lab_nodes(
    db: &db::Database,
    qemu_conn: &Connect,
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
    if requests.iter().any(|request| request.memory.is_some()) {
        let domains = libvirt::list_domain_memory(qemu_conn)?;
        check_memory(qemu_conn, settings, &requests, &pending, &domains)?;
    }
    Ok(record(requests))
}

/// Admit the stopped VMs and unikernels among `node_names` of lab `lab_id` to be started again.
/// Running and paused ones already hold their memory, and their pins stay stored while they
/// are stopped, so only the memory of stopped ones is checked.
///
/// # Errors
///
/// Fails when their memory does not fit in the server's, or libvirt cannot be read.
#[instrument(skip(qemu_conn, settings, node_names), fields(%lab_id))]
pub async fn admit_stopped_nodes(
    qemu_conn: &Connect,
    settings: HostMemorySettings,
    lab_id: &str,
    node_names: &[&str],
) -> Result<Admission> {
    let _queue = queue().lock().await;
    let pending = pending_entries();
    let domains = libvirt::list_domain_memory(qemu_conn)?;
    let requests = stopped_requests(lab_id, node_names, &domains);
    if !requests.is_empty() {
        check_memory(qemu_conn, settings, &requests, &pending, &domains)?;
    }
    Ok(record(requests))
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

/// What each node asks for: VMs and unikernels their memory, VMs their NUMA node, which holds
/// their memory strictly, and every tuned node its CPU pins.
fn lab_requests(lab_id: &str, nodes: &[(&topology::NodeExpanded, &NodeConfig)]) -> Vec<Request> {
    nodes
        .iter()
        .map(|(node, image)| {
            let tuning = node.performance_tuning.as_ref();
            let numa_node = if image.kind == NodeKind::VirtualMachine {
                tuning.and_then(|tuning| tuning.numa_node).map(u32::from)
            } else {
                None
            };
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
                pins: tuning.map(|tuning| NodeCpuPins::of(&node.name, tuning)),
            }
        })
        .collect()
}

/// The stopped domains among `node_names`, with the memory libvirt has for them.
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

/// The memory held by running domains and by nodes pending in other operations, leaving out
/// the `requested` domains, whose new memory is checked. A domain that is running and pending
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
    use libvirt::Qemu;
    use shared::data::{NodeModel, PerformanceTuning};

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
        let tuning = PerformanceTuning {
            cpu_pinning: Some(vec![2, 3]),
            numa_node: Some(1),
            ..PerformanceTuning::default()
        };
        let vm = topology::NodeExpanded {
            name: "vm".to_string(),
            memory: Some(8192),
            performance_tuning: Some(tuning.clone()),
            ..Default::default()
        };
        let unikernel = topology::NodeExpanded {
            name: "uk".to_string(),
            performance_tuning: Some(tuning),
            ..Default::default()
        };
        let container = topology::NodeExpanded {
            name: "db".to_string(),
            memory: Some(4096),
            ..Default::default()
        };
        let requests = lab_requests(
            "lab1",
            &[
                (&vm, &vm_image),
                (&unikernel, &unikernel_image),
                (&container, &container_image),
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
        assert_eq!(requests[2].memory, None);
        assert_eq!(requests[2].pins, None);
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

    /// Against the real libvirt: two nodes that each fit, but not together, are admitted one
    /// at a time, and the second only once the first is released. Needs a server with less
    /// than about 60 GiB of guest memory free, such as the dev guest; it starts nothing.
    #[tokio::test]
    #[ignore = "requires a libvirt daemon"]
    async fn a_pending_node_holds_its_memory_until_released() {
        let db = db::open_in_memory().await.unwrap();
        let conn = Qemu::default().connect().unwrap();
        let settings = HostMemorySettings::default();
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let node = |name: &str| topology::NodeExpanded {
            name: name.to_string(),
            memory: Some(30_000),
            ..Default::default()
        };
        let (first, second) = (node("first"), node("second"));

        let held = admit_lab_nodes(&db, &conn, settings, "admtest1", &[(&first, &image)])
            .await
            .unwrap();
        let refused = admit_lab_nodes(&db, &conn, settings, "admtest2", &[(&second, &image)])
            .await
            .unwrap_err();
        let message = format!("{refused:#}");
        assert!(message.contains("Memory admission failed"), "{message}");
        assert!(
            message.contains("node 'second' needs 30000 MiB"),
            "{message}"
        );
        assert!(
            message.contains("nodes other operations are starting hold 30000 MiB (1)"),
            "{message}"
        );

        drop(held);
        let admitted = admit_lab_nodes(&db, &conn, settings, "admtest2", &[(&second, &image)])
            .await
            .unwrap();
        drop(admitted);
    }
}
