//! What each node is given: vCPUs, memory and, for VMs, a VNC console port. They are decided
//! when the node is created or redeployed and stored on the node, so nothing reads them back
//! from the hypervisor.
use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};

use anyhow::{Context, Result, bail};
use tracing::instrument;

use shared::data::{DbNode, NodeConfig, NodeCpuPins, NodeKind, NodeModel, NodeResources, RecordId};
use shared::error::{CpuPinConflict, CpuPinRole};
use shared::konst::{
    CISCO_IOL_CONTAINER_OVERHEAD_MIB, JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB,
    VNC_PORT_ALLOCATION_ATTEMPTS, VNC_PORT_FIRST, VNC_PORT_LAST,
};
use virt::connect::Connect;

use crate::auth::context::AuthContext;
use crate::services::error::ServiceError;
use crate::validate::check_host_resources;

/// Pins another operation was admitted with, for a node that may not be stored yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPins {
    /// The ID of the node's lab.
    pub lab_id: String,
    /// The node's pins; `node_name` is the node's name in its lab.
    pub pins: NodeCpuPins,
}

/// A lab as a pin refusal names it, and the owner who may see it named.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PinLab {
    lab_id: String,
    name: String,
    owner: Option<String>,
}

/// A node holding host CPUs: stored on the server, or admitted to another operation and not
/// stored yet.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PinHolder {
    /// The node's lab; `None` for a stored node whose lab is not found.
    lab: Option<PinLab>,
    /// Whether another operation is starting it, rather than it being stored.
    starting: bool,
    pins: NodeCpuPins,
}

/// One host CPU two nodes both pin, against the rules: the role each pins it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PinOverlap {
    cpu: u16,
    requested: CpuPinRole,
    held: CpuPinRole,
}

/// The vCPUs and memory a node gets, and the host CPUs its `cpu` and `boot_disk` settings pin.
/// A VM or unikernel gets the manifest's values, else the image's. A container gets the
/// manifest's as Docker limits, and is not limited in what the manifest leaves out; the image's
/// values are not limits. `juniper_cevolved` sizes its EVO VM from the manifest or image values,
/// so its container's memory limit is that and its overhead, with no CPU quota, which would
/// throttle the EVO VM. IOL takes the manifest or image memory as its own, so its container's
/// limit is that and the launcher's overhead. The VNC port is reserved separately.
pub fn planned_resources(node: &topology::NodeExpanded, image: &NodeConfig) -> NodeResources {
    match image.kind {
        NodeKind::VirtualMachine | NodeKind::Unikernel => {
            let pins = NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk);
            NodeResources {
                cpu_count: Some(node.cpu.count.unwrap_or(image.cpu.count)),
                memory: Some(node.memory.unwrap_or(image.memory)),
                vnc_port: None,
                cpu_pinning: node.cpu.pinning.clone(),
                qemu_thread_cpus: Some(pins.qemu_thread_cpus).filter(|cpus| !cpus.is_empty()),
            }
        }
        NodeKind::Container if image.model == NodeModel::JuniperCevolved => NodeResources {
            // `check_ptx_platform` leaves room for the overhead.
            memory: Some(
                node.memory
                    .unwrap_or(image.memory)
                    .saturating_add(JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB),
            ),
            ..NodeResources::default()
        },
        NodeKind::Container
            if matches!(image.model, NodeModel::CiscoIol | NodeModel::CiscoIoll2) =>
        {
            NodeResources {
                // IOL's own memory, its `-m`, is the manifest or image value; the launcher and
                // the bridge need the overhead on top.
                cpu_count: node.cpu.count,
                memory: Some(
                    node.memory
                        .unwrap_or(image.memory)
                        .saturating_add(CISCO_IOL_CONTAINER_OVERHEAD_MIB),
                ),
                ..NodeResources::default()
            }
        }
        NodeKind::Container => NodeResources {
            cpu_count: node.cpu.count,
            memory: node.memory,
            ..NodeResources::default()
        },
    }
}

/// The resources stored on a node.
pub fn resources_of(node: &DbNode) -> NodeResources {
    NodeResources {
        cpu_count: node.cpu_count,
        memory: node.memory,
        vnc_port: node.vnc_port,
        cpu_pinning: node.cpu_pinning.clone(),
        qemu_thread_cpus: node.qemu_thread_cpus.clone(),
    }
}

/// Check the CPU pins and NUMA node of `nodes`, in lab `lab_id`, against the server: the host
/// CPUs and NUMA nodes they name must exist, and their pins must be free (see
/// [`check_cpu_pins_free`]). Returns a warning for each overlap the nodes' stored pins already
/// had.
#[instrument(skip(db, qemu_conn, requester, nodes, pending), level = "debug")]
pub async fn check_cpu_pins_on_server(
    db: &db::Database,
    qemu_conn: &Connect,
    requester: &AuthContext,
    lab_id: &str,
    nodes: &[&topology::NodeExpanded],
    pending: &[PendingPins],
) -> Result<Vec<String>> {
    let tuned: Vec<(&topology::NodeExpanded, NodeCpuPins)> = nodes
        .iter()
        .map(|node| {
            (
                *node,
                NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk),
            )
        })
        .filter(|(node, pins)| !pins.is_empty() || node.numa.node.is_some())
        .collect();
    if tuned.is_empty() {
        return Ok(Vec::new());
    }

    let host = libvirt::host_cpus(qemu_conn)?;
    let mut pins = Vec::new();
    for (node, node_pins) in tuned {
        check_host_resources(node, host.cpus, host.numa_nodes)?;
        pins.push(node_pins);
    }
    check_cpu_pins_free(db, requester, lab_id, &pins, pending).await
}

/// Check that the host CPUs the requested `nodes` of lab `lab_id` pin are free: no other
/// node, stored on the server in any lab or `pending` in another operation, may pin one of
/// them to its vCPUs, nor pin one of their vCPU CPUs to its QEMU threads. A requested node's
/// own stored and pending pins are its own, since it is being replaced. Every other node's
/// stored pins count, also while another operation redeploys it: its old domain may still run
/// on them, and keeps them if the redeploy fails.
///
/// An overlap the requested node's stored pins already had with the same node, against the
/// rules, is carried and returned as a warning, so a lab that already shares CPUs still starts.
/// A new overlap is refused with [`ServiceError::CpuPinsHeld`], naming the holding node and its
/// lab to admins and to that lab's owner, and "another lab" to everyone else.
#[instrument(skip(db, requester, nodes, pending), fields(username = %requester.username), level = "debug")]
pub async fn check_cpu_pins_free(
    db: &db::Database,
    requester: &AuthContext,
    lab_id: &str,
    nodes: &[NodeCpuPins],
    pending: &[PendingPins],
) -> Result<Vec<String>> {
    if nodes.iter().all(NodeCpuPins::is_empty) {
        return Ok(Vec::new());
    }
    let users: HashMap<RecordId, String> = db::list_users(db)
        .await?
        .into_iter()
        .filter_map(|user| user.id.map(|id| (id, user.username)))
        .collect();
    let labs: Vec<(RecordId, PinLab)> = db::list_labs(db)
        .await?
        .into_iter()
        .filter_map(|lab| {
            let owner = users.get(&lab.user).cloned();
            lab.id.map(|id| {
                (
                    id,
                    PinLab {
                        lab_id: lab.lab_id,
                        name: lab.name,
                        owner,
                    },
                )
            })
        })
        .collect();
    let mut holders: Vec<PinHolder> = db::list_pinned_nodes(db)
        .await?
        .into_iter()
        .map(|node| PinHolder {
            lab: labs
                .iter()
                .find(|(id, _)| *id == node.lab)
                .map(|(_, lab)| lab.clone()),
            starting: false,
            pins: NodeCpuPins {
                node_name: node.name,
                vcpu_cpus: node.cpu_pinning.unwrap_or_default(),
                qemu_thread_cpus: node.qemu_thread_cpus.unwrap_or_default(),
            },
        })
        .collect();
    holders.extend(pending.iter().map(|entry| {
        PinHolder {
            lab: Some(
                labs.iter()
                    .find(|(_, lab)| lab.lab_id == entry.lab_id)
                    .map_or_else(
                        || PinLab {
                            lab_id: entry.lab_id.clone(),
                            name: entry.lab_id.clone(),
                            owner: None,
                        },
                        |(_, lab)| lab.clone(),
                    ),
            ),
            starting: true,
            pins: entry.pins.clone(),
        }
    }));
    check_pins_against_holders(requester, lab_id, nodes, &holders)
}

/// Check the requested `nodes` of lab `lab_id` against the `holders`; see
/// [`check_cpu_pins_free`].
fn check_pins_against_holders(
    requester: &AuthContext,
    lab_id: &str,
    nodes: &[NodeCpuPins],
    holders: &[PinHolder],
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for node in nodes.iter().filter(|node| !node.is_empty()) {
        let own_stored = holders
            .iter()
            .find(|holder| !holder.starting && holder.is(lab_id, &node.node_name));
        for holder in holders
            .iter()
            .filter(|holder| !holder.is(lab_id, &node.node_name))
        {
            let overlaps = pin_overlaps(node, &holder.pins);
            if overlaps.is_empty() {
                continue;
            }
            let carried = carried_overlaps(own_stored, holder, holders);
            let (old, new): (Vec<PinOverlap>, Vec<PinOverlap>) = overlaps
                .into_iter()
                .partition(|overlap| carried.contains(overlap));
            if let Some(first) = new.first() {
                return Err(pins_held(requester, &node.node_name, holder, *first, &new).into());
            }
            let warning = shared_cpus_warning(requester, &node.node_name, holder, &old);
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    Ok(warnings)
}

impl PinHolder {
    /// Whether this is node `node_name` of lab `lab_id`.
    fn is(&self, lab_id: &str, node_name: &str) -> bool {
        self.pins.node_name == node_name
            && self.lab.as_ref().is_some_and(|lab| lab.lab_id == lab_id)
    }

    /// The node and its lab, as `requester` may see them: named to admins and to the lab's
    /// owner, "another lab" to everyone else.
    fn describe(&self, requester: &AuthContext) -> String {
        let visible = requester.is_admin
            || self
                .lab
                .as_ref()
                .and_then(|lab| lab.owner.as_deref())
                .is_some_and(|owner| owner == requester.username);
        if !visible {
            return "a node of another lab".to_string();
        }
        let lab = self.lab.as_ref().map_or_else(
            || "a lab that is not found".to_string(),
            |lab| format!("lab '{}' ({})", lab.name, lab.lab_id),
        );
        let starting = if self.starting {
            ", which another operation is starting"
        } else {
            ""
        };
        format!("node '{}' of {lab}{starting}", self.pins.node_name)
    }
}

/// The host CPUs `requested` pins against the rules beside `held`: a vCPU CPU the other also
/// pins, to its vCPUs or its QEMU threads, or a QEMU thread CPU the other pins to its vCPUs.
/// QEMU threads may share CPUs. Sorted, one per CPU and pair of roles.
fn pin_overlaps(requested: &NodeCpuPins, held: &NodeCpuPins) -> Vec<PinOverlap> {
    let mut overlaps = Vec::new();
    for &cpu in &requested.vcpu_cpus {
        if held.vcpu_cpus.contains(&cpu) {
            overlaps.push(PinOverlap {
                cpu,
                requested: CpuPinRole::Vcpu,
                held: CpuPinRole::Vcpu,
            });
        } else if held.qemu_thread_cpus.contains(&cpu) {
            overlaps.push(PinOverlap {
                cpu,
                requested: CpuPinRole::Vcpu,
                held: CpuPinRole::Thread,
            });
        }
    }
    for &cpu in &requested.qemu_thread_cpus {
        if held.vcpu_cpus.contains(&cpu) {
            overlaps.push(PinOverlap {
                cpu,
                requested: CpuPinRole::Thread,
                held: CpuPinRole::Vcpu,
            });
        }
    }
    overlaps.sort_unstable();
    overlaps.dedup();
    overlaps
}

/// The overlaps the requested node's stored pins, `own_stored`, already had with the stored
/// pins of the same node as `holder`. They were stored before the rules were enforced
/// everywhere, and are carried.
fn carried_overlaps(
    own_stored: Option<&PinHolder>,
    holder: &PinHolder,
    holders: &[PinHolder],
) -> Vec<PinOverlap> {
    let Some(own_stored) = own_stored else {
        return Vec::new();
    };
    let Some(holder_lab) = holder.lab.as_ref() else {
        return pin_overlaps(&own_stored.pins, &holder.pins);
    };
    holders
        .iter()
        .filter(|other| !other.starting && other.is(&holder_lab.lab_id, &holder.pins.node_name))
        .flat_map(|other| pin_overlaps(&own_stored.pins, &other.pins))
        .collect()
}

/// The refusal for node `node_name`, whose pins overlap `holder`'s on the `overlaps`: the
/// host CPUs of the `first` overlap's pair of roles.
fn pins_held(
    requester: &AuthContext,
    node_name: &str,
    holder: &PinHolder,
    first: PinOverlap,
    overlaps: &[PinOverlap],
) -> ServiceError {
    let host_cpus: Vec<u16> = overlaps
        .iter()
        .filter(|overlap| overlap.requested == first.requested && overlap.held == first.held)
        .map(|overlap| overlap.cpu)
        .collect();
    let message = format!(
        "{}, which node '{node_name}' would pin to its {}, {} pinned to the {} of {}. Try again \
         once that node is destroyed or redeployed without them",
        cpu_list(&host_cpus),
        first.requested.describe(),
        if host_cpus.len() == 1 { "is" } else { "are" },
        first.held.describe(),
        holder.describe(requester),
    );
    ServiceError::CpuPinsHeld {
        message,
        conflict: CpuPinConflict {
            host_cpus,
            requested_role: first.requested,
            held_role: first.held,
        },
    }
}

/// The warning for node `node_name`, whose stored pins already overlap `holder`'s on the
/// `overlaps`.
fn shared_cpus_warning(
    requester: &AuthContext,
    node_name: &str,
    holder: &PinHolder,
    overlaps: &[PinOverlap],
) -> String {
    let host_cpus: Vec<u16> = overlaps.iter().map(|overlap| overlap.cpu).collect();
    format!(
        "{} of node '{node_name}' {} also pinned by {}, against the CPU pinning rules. Starting \
         the node anyway; redeploy one of the two on other host CPUs to fix it",
        cpu_list(&host_cpus),
        if host_cpus.len() == 1 { "is" } else { "are" },
        holder.describe(requester),
    )
}

/// "Host CPU 2" or "Host CPUs 2, 3".
fn cpu_list(cpus: &[u16]) -> String {
    let listed: Vec<String> = cpus.iter().map(u16::to_string).collect();
    let noun = if cpus.len() == 1 {
        "Host CPU"
    } else {
        "Host CPUs"
    };
    format!("{noun} {}", listed.join(", "))
}

/// The libvirt domain name of node `node_name` in lab `lab_id`.
pub fn domain_name(node_name: &str, lab_id: &str) -> String {
    format!("{node_name}-{lab_id}")
}

/// VNC ports in the reserved range that no node holds, lowest first.
pub fn free_vnc_ports(taken: &[u16]) -> Vec<u16> {
    let mut free = Vec::new();
    for port in VNC_PORT_FIRST..=VNC_PORT_LAST {
        if taken.binary_search(&port).is_err() {
            free.push(port);
        }
    }
    free
}

/// Store a node's resources. A VM keeps the VNC port it has, or reserves the lowest port no
/// node holds and nothing on the server listens on (QEMU listens on 127.0.0.1). The port's
/// unique index settles a race with another lab, and the reservation is retried.
#[instrument(skip(db, planned), fields(node = %node.name), level = "debug")]
pub async fn reserve_node_resources(
    db: &db::Database,
    node: &DbNode,
    kind: &NodeKind,
    planned: NodeResources,
) -> Result<DbNode> {
    let id: RecordId = node
        .id
        .clone()
        .with_context(|| format!("Node '{}' has no record id", node.name))?;
    if *kind != NodeKind::VirtualMachine || node.vnc_port.is_some() {
        let vnc_port = if *kind == NodeKind::VirtualMachine {
            node.vnc_port
        } else {
            None
        };
        let resources = NodeResources {
            vnc_port,
            ..planned
        };
        return db::update_node_resources(db, id, &resources).await;
    }

    for attempt in 1..=VNC_PORT_ALLOCATION_ATTEMPTS {
        let taken = db::list_vnc_ports(db).await?;
        let mut port = None;
        for candidate in free_vnc_ports(&taken) {
            if host_port_free(candidate) {
                port = Some(candidate);
                break;
            }
        }
        let Some(port) = port else {
            bail!("No free VNC port between {VNC_PORT_FIRST} and {VNC_PORT_LAST}");
        };
        let resources = NodeResources {
            vnc_port: Some(port),
            ..planned.clone()
        };
        match db::update_node_resources(db, id.clone(), &resources).await {
            Ok(updated) => return Ok(updated),
            Err(error) => {
                if !db::list_vnc_ports(db).await?.contains(&port) {
                    return Err(error);
                }
                tracing::warn!(node = %node.name, port, attempt, "Another node took the VNC port; reserving again");
            }
        }
    }
    bail!(
        "Could not reserve a VNC port for node '{}' after {VNC_PORT_ALLOCATION_ATTEMPTS} attempts",
        node.name
    )
}

/// Whether nothing on the server listens on a port.
fn host_port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeCpu;

    fn node(cpu_count: Option<u8>, memory: Option<u16>) -> topology::NodeExpanded {
        topology::NodeExpanded {
            name: "dev01".to_string(),
            cpu: NodeCpu {
                count: cpu_count,
                ..NodeCpu::default()
            },
            memory,
            ..Default::default()
        }
    }

    #[test]
    fn vms_and_unikernels_get_the_manifest_values_else_the_images() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        assert_eq!(
            planned_resources(&node(Some(16), Some(32768)), &image),
            NodeResources {
                cpu_count: Some(16),
                memory: Some(32768),
                ..NodeResources::default()
            }
        );
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(defaults.cpu_count, Some(image.cpu.count));
        assert_eq!(defaults.memory, Some(image.memory));

        let unikernel = NodeConfig::get_model(NodeModel::NanosUnikernel);
        assert_eq!(
            planned_resources(&node(Some(2), None), &unikernel).cpu_count,
            Some(2)
        );
    }

    #[test]
    fn a_tuned_vm_gets_its_pinned_cpus() {
        let image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        let mut tuned = node(Some(2), None);
        tuned.cpu.pinning = Some(vec![9, 25]);
        tuned.cpu.emulator_pinning = Some(vec![24, 8]);
        tuned.boot_disk.io_thread_cpus = Some(vec![8]);
        let resources = planned_resources(&tuned, &image);
        assert_eq!(resources.cpu_pinning, Some(vec![9, 25]));
        assert_eq!(resources.qemu_thread_cpus, Some(vec![8, 24]));

        let unpinned = planned_resources(&node(Some(2), None), &image);
        assert_eq!(unpinned.cpu_pinning, None);
        assert_eq!(unpinned.qemu_thread_cpus, None);
    }

    #[test]
    fn containers_get_the_manifest_limits_only() {
        let image = NodeConfig::get_model(NodeModel::MongoDb);
        assert_eq!(
            planned_resources(&node(Some(4), Some(4096)), &image),
            NodeResources {
                cpu_count: Some(4),
                memory: Some(4096),
                ..NodeResources::default()
            }
        );
        // The image's values are not limits: an unset container is not limited.
        assert_eq!(
            planned_resources(&node(None, None), &image),
            NodeResources::default()
        );
    }

    #[test]
    fn cevolved_gets_its_evo_vm_memory_and_overhead_and_no_cpu_quota() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(
            defaults.memory,
            Some(image.memory + JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB)
        );
        assert_eq!(defaults.cpu_count, None);
        let sized = planned_resources(&node(Some(8), Some(16384)), &image);
        assert_eq!(sized.memory, Some(16384 + 2048));
        assert_eq!(sized.cpu_count, None);
    }

    #[test]
    fn iol_gets_its_memory_and_overhead_as_its_limit_and_the_manifest_cpus() {
        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            let image = NodeConfig::get_model(model);
            let defaults = planned_resources(&node(None, None), &image);
            assert_eq!(
                defaults.memory,
                Some(image.memory + CISCO_IOL_CONTAINER_OVERHEAD_MIB),
                "{model}"
            );
            assert_eq!(defaults.cpu_count, None, "{model}");
            let sized = planned_resources(&node(Some(2), Some(1024)), &image);
            assert_eq!(sized.memory, Some(1024 + 128), "{model}");
            assert_eq!(sized.cpu_count, Some(2), "{model}");
        }
    }

    fn ctx(username: &str, is_admin: bool) -> AuthContext {
        AuthContext::new(username.to_string(), is_admin, "session".to_string())
    }

    fn pins(node_name: &str, vcpus: &[u16], threads: &[u16]) -> NodeCpuPins {
        NodeCpuPins {
            node_name: node_name.to_string(),
            vcpu_cpus: vcpus.to_vec(),
            qemu_thread_cpus: threads.to_vec(),
        }
    }

    fn pending(lab_id: &str, node: NodeCpuPins) -> PendingPins {
        PendingPins {
            lab_id: lab_id.to_string(),
            pins: node,
        }
    }

    /// A database with users alice and bob, and labs, each with one node: `lab-a` (`a00000a1`)
    /// of alice, its node `n1` pinning vCPUs 2 and 3 and thread CPU 8, and `lab-b` (`b00000b1`)
    /// of bob, its node `n1` pinning nothing.
    async fn server() -> db::Database {
        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        let image = db::upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            db::DefaultRequest::Keep,
        )
        .await
        .unwrap()
        .id
        .unwrap();
        for (index, (username, lab_id, name)) in
            [("alice", "a00000a1", "lab-a"), ("bob", "b00000b1", "lab-b")]
                .into_iter()
                .enumerate()
        {
            let user = db::create_user(&db, username.to_string(), "TestPass123!", false, vec![])
                .await
                .unwrap();
            let networks = db::LabNetworks {
                loopback_network: format!("127.127.{index}.0/24"),
                management_network: format!("172.31.{index}.0/24"),
                gateway_ipv4: format!("172.31.{index}.1"),
                router_ipv4: format!("172.31.{index}.2"),
                management_network_v6: None,
                loopback_network_v6: None,
                gateway_ipv6: None,
                router_ipv6: None,
            };
            let lab = db::create_lab(
                &db,
                name,
                lab_id,
                &user,
                &networks,
                &db::LabDetails::default(),
            )
            .await
            .unwrap();
            db::create_node(&db, "n1", 1, image.clone(), lab.id.unwrap())
                .await
                .unwrap();
        }
        store_pins(&db, "a00000a1", "n1", &[2, 3], &[8]).await;
        db
    }

    /// Store pins on node `node_name` of lab `lab_id`, as a successful `up` or redeploy does.
    async fn store_pins(
        db: &db::Database,
        lab_id: &str,
        node_name: &str,
        vcpus: &[u16],
        threads: &[u16],
    ) {
        let lab = db::get_lab(db, lab_id).await.unwrap();
        let node = db::get_node_by_name_and_lab(db, node_name, db::get_lab_id(&lab).unwrap())
            .await
            .unwrap();
        let resources = NodeResources {
            cpu_pinning: Some(vcpus.to_vec()).filter(|cpus| !cpus.is_empty()),
            qemu_thread_cpus: Some(threads.to_vec()).filter(|cpus| !cpus.is_empty()),
            ..NodeResources::default()
        };
        db::update_node_resources(db, node.id.unwrap(), &resources)
            .await
            .unwrap();
    }

    /// The refusal of a pin check, which must be the typed one.
    fn refusal(result: Result<Vec<String>>) -> (String, CpuPinConflict) {
        let error = result.expect_err("the pins are refused");
        match ServiceError::find(&error) {
            Some(ServiceError::CpuPinsHeld { message, conflict }) => {
                (message.clone(), conflict.clone())
            }
            _ => panic!("not a CpuPinsHeld refusal: {error:#}"),
        }
    }

    #[tokio::test]
    async fn a_stored_nodes_vcpu_cpus_are_refused_to_another_lab() {
        let db = server().await;
        let (message, conflict) = refusal(
            check_cpu_pins_free(
                &db,
                &ctx("bob", false),
                "b00000b1",
                &[pins("n1", &[3, 4], &[])],
                &[],
            )
            .await,
        );
        assert_eq!(
            conflict,
            CpuPinConflict {
                host_cpus: vec![3],
                requested_role: CpuPinRole::Vcpu,
                held_role: CpuPinRole::Vcpu,
            }
        );
        assert_eq!(
            message,
            "Host CPU 3, which node 'n1' would pin to its vCPUs, is pinned to the vCPUs of a \
             node of another lab. Try again once that node is destroyed or redeployed without \
             them"
        );
    }

    #[tokio::test]
    async fn vcpus_and_threads_may_not_share_but_threads_may() {
        let db = server().await;
        let bob = ctx("bob", false);
        let (_, conflict) = refusal(
            check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[8], &[])], &[]).await,
        );
        assert_eq!(
            (conflict.requested_role, conflict.held_role),
            (CpuPinRole::Vcpu, CpuPinRole::Thread)
        );
        let (_, conflict) = refusal(
            check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[5], &[2, 3])], &[]).await,
        );
        assert_eq!(conflict.host_cpus, vec![2, 3]);
        assert_eq!(
            (conflict.requested_role, conflict.held_role),
            (CpuPinRole::Thread, CpuPinRole::Vcpu)
        );
        let warnings = check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[5], &[8])], &[])
            .await
            .unwrap();
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[tokio::test]
    async fn admins_and_the_holding_labs_owner_see_the_holder_and_others_see_another_lab() {
        let db = server().await;
        let request = [pins("n2", &[2], &[])];
        let other =
            refusal(check_cpu_pins_free(&db, &ctx("bob", false), "b00000b1", &request, &[]).await)
                .0;
        assert!(other.contains("of a node of another lab."), "{other}");
        assert!(
            !other.contains("lab-a") && !other.contains("a00000a1"),
            "{other}"
        );

        let holder = "the vCPUs of node 'n1' of lab 'lab-a' (a00000a1).";
        let admin =
            refusal(check_cpu_pins_free(&db, &ctx("carol", true), "b00000b1", &request, &[]).await)
                .0;
        assert!(admin.contains(holder), "{admin}");
        // Alice owns the holding lab, and asks for its CPUs for another of her labs.
        let owner = refusal(
            check_cpu_pins_free(&db, &ctx("alice", false), "c00000c1", &request, &[]).await,
        )
        .0;
        assert!(owner.contains(holder), "{owner}");
    }

    #[tokio::test]
    async fn a_node_being_started_holds_its_pins_and_is_named_so() {
        let db = server().await;
        let started = [pending("b00000b1", pins("n1", &[6], &[]))];
        let (message, conflict) = refusal(
            check_cpu_pins_free(
                &db,
                &ctx("carol", true),
                "c00000c1",
                &[pins("x", &[6], &[])],
                &started,
            )
            .await,
        );
        assert_eq!(conflict.host_cpus, vec![6]);
        assert!(
            message.contains(
                "node 'n1' of lab 'lab-b' (b00000b1), which another operation is starting."
            ),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_requested_node_does_not_overlap_its_own_stored_or_pending_pins() {
        let db = server().await;
        let own = [pending("a00000a1", pins("n1", &[2], &[]))];
        let warnings = check_cpu_pins_free(
            &db,
            &ctx("alice", false),
            "a00000a1",
            &[pins("n1", &[2, 3], &[8])],
            &own,
        )
        .await
        .unwrap();
        assert_eq!(warnings, Vec::<String>::new());
    }

    /// The redeploy of lab-a's `n1` from vCPUs 2 and 3 to 4 and 5, step by step: what bob's lab
    /// may pin at each point, as the stored row and the ledger then are.
    #[tokio::test]
    async fn a_redeploying_node_holds_its_old_and_new_pins_until_it_ends() {
        let db = server().await;
        let bob = ctx("bob", false);
        let old = [pins("n1", &[2, 3], &[])];
        let new = [pins("n1", &[4, 5], &[])];
        let redeploying = [pending("a00000a1", pins("n1", &[4, 5], &[8]))];

        // Admitted, before and after teardown, before its new pins are stored: both are held.
        for wanted in [&old, &new] {
            refusal(check_cpu_pins_free(&db, &bob, "b00000b1", wanted, &redeploying).await);
        }

        // It failed before storing the new pins: the old ones stay held, the new are free.
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &old, &[]).await);
        check_cpu_pins_free(&db, &bob, "b00000b1", &new, &[])
            .await
            .unwrap();

        // The new pins were stored, and the redeploy then failed or succeeded: only the new ones
        // are held.
        store_pins(&db, "a00000a1", "n1", &[4, 5], &[8]).await;
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &new, &redeploying).await);
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &new, &[]).await);
        check_cpu_pins_free(&db, &bob, "b00000b1", &old, &[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn destroying_a_lab_lets_its_pins_go() {
        let db = server().await;
        let bob = ctx("bob", false);
        let wanted = [pins("n1", &[2, 3], &[])];
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &wanted, &[]).await);
        assert!(
            db::delete_lab_from(&db, "a00000a1", shared::data::LabPhase::Creating)
                .await
                .unwrap()
        );
        check_cpu_pins_free(&db, &bob, "b00000b1", &wanted, &[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_overlap_already_stored_is_a_warning_and_a_new_one_is_refused() {
        let db = server().await;
        // Stored before the rules were enforced everywhere: both labs' n1 on host CPU 3.
        store_pins(&db, "b00000b1", "n1", &[3], &[]).await;
        let warnings = check_cpu_pins_free(
            &db,
            &ctx("bob", false),
            "b00000b1",
            &[pins("n1", &[3], &[])],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(
            warnings,
            [
                "Host CPU 3 of node 'n1' is also pinned by a node of another lab, against the CPU \
              pinning rules. Starting the node anyway; redeploy one of the two on other host \
              CPUs to fix it"
            ]
        );
        let admin = check_cpu_pins_free(
            &db,
            &ctx("carol", true),
            "a00000a1",
            &[pins("n1", &[2, 3], &[8])],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(admin.len(), 1);
        assert!(
            admin[0].contains("node 'n1' of lab 'lab-b' (b00000b1)"),
            "{admin:?}"
        );

        // A redeploy that keeps the shared CPU but takes one more of the other node's is
        // refused for that one only.
        let (_, conflict) = refusal(
            check_cpu_pins_free(
                &db,
                &ctx("bob", false),
                "b00000b1",
                &[pins("n1", &[2, 3], &[])],
                &[],
            )
            .await,
        );
        assert_eq!(conflict.host_cpus, vec![2]);
    }

    #[test]
    fn overlaps_list_each_cpu_and_pair_of_roles_once() {
        let overlaps = pin_overlaps(
            &pins("a", &[3, 1, 3, 9], &[8, 4]),
            &pins("b", &[1, 3, 4], &[9]),
        );
        let found: Vec<(u16, CpuPinRole, CpuPinRole)> = overlaps
            .iter()
            .map(|overlap| (overlap.cpu, overlap.requested, overlap.held))
            .collect();
        assert_eq!(
            found,
            [
                (1, CpuPinRole::Vcpu, CpuPinRole::Vcpu),
                (3, CpuPinRole::Vcpu, CpuPinRole::Vcpu),
                (4, CpuPinRole::Thread, CpuPinRole::Vcpu),
                (9, CpuPinRole::Vcpu, CpuPinRole::Thread),
            ]
        );
    }

    #[test]
    fn free_ports_skip_taken_ones_and_stay_in_the_range() {
        let free = free_vnc_ports(&[5900, 5902]);
        assert_eq!(&free[..3], &[5901, 5903, 5904]);
        assert_eq!(free.last(), Some(&VNC_PORT_LAST));
        assert_eq!(
            free.len(),
            usize::from(VNC_PORT_LAST - VNC_PORT_FIRST + 1) - 2
        );
    }
}
