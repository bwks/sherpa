//! What each node is given: vCPUs and memory. They are decided
//! when the node is created or redeployed and stored on the node, so nothing reads them back
//! from the hypervisor. The host CPUs a node pins are in the `cpu_pin` table, claimed at
//! admission; this module checks a request's pins against it.
use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{
    DbCpuPins, DbNode, LabPhase, NodeConfig, NodeCpuPins, NodeKind, NodeModel, NodeResources,
    RecordId,
};
use shared::error::{CpuPinConflict, CpuPinRole};
use shared::konst::JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB;
use virt::connect::Connect;

use crate::auth::context::AuthContext;
use crate::services::error::ServiceError;
use crate::validate::check_host_resources;

/// The memory, in MiB, an IOL container needs beyond IOL's own (its `-m`) for the launcher and
/// the bridge: its Docker memory limit is IOL's memory plus this.
const CISCO_IOL_CONTAINER_OVERHEAD_MIB: u16 = 128;

/// A lab as a pin refusal names it, and the owner who may see it named.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PinLab {
    lab_id: String,
    name: String,
    owner: Option<String>,
}

/// A node holding host CPUs in the `cpu_pin` table.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PinHolder {
    lab: Option<PinLab>,
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

/// The vCPUs and memory a node gets. A VM or unikernel gets the manifest's values, else the image's. A container gets the
/// manifest's as Docker limits, and is not limited in what the manifest leaves out; the image's
/// values are not limits. a `juniper_evolved` container sizes its EVO VM from the manifest or image values,
/// so its container's memory limit is that and its overhead, with no CPU quota, which would
/// throttle the EVO VM. IOL takes the manifest or image memory as its own, so its container's
/// limit is that and the launcher's overhead.
pub fn planned_resources(node: &topology::NodeExpanded, image: &NodeConfig) -> NodeResources {
    match image.kind() {
        NodeKind::VirtualMachine | NodeKind::Unikernel => NodeResources {
            cpu_count: Some(node.cpu.count.unwrap_or(image.cpu.count)),
            memory: Some(node.memory.unwrap_or(image.memory)),
        },
        NodeKind::Container if image.model == NodeModel::JuniperEvolved => NodeResources {
            // `check_ptx_platform` leaves room for the overhead.
            memory: Some(
                node.memory
                    .unwrap_or(image.memory)
                    .saturating_add(JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB),
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
            }
        }
        NodeKind::Container => NodeResources {
            cpu_count: node.cpu.count,
            memory: node.memory,
        },
    }
}

/// The resources stored on a node.
pub fn resources_of(node: &DbNode) -> NodeResources {
    NodeResources {
        cpu_count: node.cpu_count,
        memory: node.memory,
    }
}

/// Check the CPU pins and NUMA node of `nodes`, in lab `lab_id`, against the server: the host
/// CPUs and NUMA nodes they name must exist, and their pins must be free (see
/// [`check_cpu_pins_free`]). Returns a warning for each overlap the nodes' stored pins already
/// had.
#[instrument(skip(db, qemu_conn, requester, nodes), level = "debug")]
pub async fn check_cpu_pins_on_server(
    db: &db::Database,
    qemu_conn: &Connect,
    requester: &AuthContext,
    lab_id: &str,
    nodes: &[&topology::NodeExpanded],
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
    check_cpu_pins_free(db, requester, lab_id, &pins).await
}

/// Check that the host CPUs the requested `nodes` of lab `lab_id` pin are free: no node of
/// another lab or of another name in the `cpu_pin` table may pin one of them to its vCPUs, nor
/// pin one of their vCPU CPUs to its QEMU threads. A requested node's own rows are its own,
/// since it is being replaced. Every other node's rows count, also while another operation
/// redeploys it: it holds its old and its new pins until that redeploy keeps one set.
///
/// An overlap the requested node's own rows already had with the same node, against the
/// rules, is carried and returned as a warning, so a lab that already shares CPUs still starts.
/// A new overlap is refused with [`ServiceError::CpuPinsHeld`], naming the holding node and its
/// lab to admins and to that lab's owner, and "another lab" to everyone else. The table's
/// trigger refuses the same overlaps when the pins are claimed; this check comes first to name
/// the holder.
#[instrument(skip(db, requester, nodes), fields(username = %requester.username), level = "debug")]
pub async fn check_cpu_pins_free(
    db: &db::Database,
    requester: &AuthContext,
    lab_id: &str,
    nodes: &[NodeCpuPins],
) -> Result<Vec<String>> {
    if nodes.iter().all(NodeCpuPins::is_empty) {
        return Ok(Vec::new());
    }
    let holders = pin_holders(db).await?;
    check_pins_against_holders(requester, lab_id, nodes, &holders)
}

/// Every node in the `cpu_pin` table, with its lab as a refusal names it.
async fn pin_holders(db: &db::Database) -> Result<Vec<PinHolder>> {
    let users: HashMap<RecordId, String> = db::list_users(db)
        .await?
        .into_iter()
        .filter_map(|user| user.id.map(|id| (id, user.username)))
        .collect();
    let labs: Vec<(RecordId, LabPhase, PinLab)> = db::list_labs(db)
        .await?
        .into_iter()
        .filter_map(|lab| {
            let owner = users.get(&lab.user).cloned();
            lab.id.map(|id| {
                (
                    id,
                    lab.phase,
                    PinLab {
                        lab_id: lab.lab_id,
                        name: lab.name,
                        owner,
                    },
                )
            })
        })
        .collect();
    Ok(db::list_cpu_pins(db)
        .await?
        .into_iter()
        .map(|DbCpuPins { lab, pins }| {
            let found = labs.iter().find(|(id, _, _)| *id == lab);
            PinHolder {
                lab: found.map(|(_, _, lab)| lab.clone()),
                starting: found.is_some_and(|(_, phase, _)| *phase == LabPhase::Creating),
                pins,
            }
        })
        .collect())
}

/// Log a warning for each pair of nodes in the `cpu_pin` table that share a host CPU against
/// the pinning rules: pins stored before the table enforced them. Both nodes keep running; a
/// redeploy of one of them on other host CPUs ends the overlap.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db))]
pub async fn warn_shared_cpu_pins(db: &db::Database) -> Result<()> {
    let holders = pin_holders(db).await?;
    for (index, holder) in holders.iter().enumerate() {
        for other in &holders[index + 1..] {
            let overlaps = pin_overlaps(&holder.pins, &other.pins);
            if overlaps.is_empty() {
                continue;
            }
            let host_cpus: Vec<u16> = overlaps.iter().map(|overlap| overlap.cpu).collect();
            tracing::warn!(
                "{} of {} {} also pinned by {}, against the CPU pinning rules; redeploy one of \
                 the two on other host CPUs to fix it",
                cpu_list(&host_cpus),
                holder.name_for_admins(),
                if host_cpus.len() == 1 { "is" } else { "are" },
                other.name_for_admins(),
            );
        }
    }
    Ok(())
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
            .find(|holder| holder.is(lab_id, &node.node_name));
        for holder in holders
            .iter()
            .filter(|holder| !holder.is(lab_id, &node.node_name))
        {
            let overlaps = pin_overlaps(node, &holder.pins);
            if overlaps.is_empty() {
                continue;
            }
            // Overlaps the node's own rows already had with this holder were stored before the
            // rules were enforced everywhere, and are carried.
            let carried =
                own_stored.map_or_else(Vec::new, |own| pin_overlaps(&own.pins, &holder.pins));
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
        if visible {
            self.name_for_admins()
        } else {
            "a node of another lab".to_string()
        }
    }

    /// The node and its lab, named.
    fn name_for_admins(&self) -> String {
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

/// Store the resources `planned` for node `node`.
///
/// # Errors
///
/// Fails when the node has no record ID or the database cannot be written.
#[instrument(skip(db, planned), fields(node = %node.name), level = "debug")]
pub async fn store_node_resources(
    db: &db::Database,
    node: &DbNode,
    planned: NodeResources,
) -> Result<DbNode> {
    let id: RecordId = node
        .id
        .clone()
        .with_context(|| format!("Node '{}' has no record id", node.name))?;
    db::update_node_resources(db, id, &planned).await
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;
    use db::LabDetails;
    use shared::data::{DbLab, DbUser, ModelKind, NodeCpu};

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
    fn vm_start_008_a_vm_runs_with_the_manifests_memory_and_cpus_else_its_images() {
        let image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        assert_eq!(
            planned_resources(&node(Some(16), Some(32768)), &image),
            NodeResources {
                cpu_count: Some(16),
                memory: Some(32768),
            }
        );
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(defaults.cpu_count, Some(image.cpu.count));
        assert_eq!(defaults.memory, Some(image.memory));

        let unikernel = ModelKind::default_for(NodeModel::NanosUnikernel).node_config();
        assert_eq!(
            planned_resources(&node(Some(2), None), &unikernel).cpu_count,
            Some(2)
        );
    }

    #[test]
    fn container_run_009_a_container_that_sets_no_limits_runs_without_them() {
        let image = ModelKind::default_for(NodeModel::MongoDb).node_config();
        assert_eq!(
            planned_resources(&node(Some(4), Some(4096)), &image),
            NodeResources {
                cpu_count: Some(4),
                memory: Some(4096),
            }
        );
        // The image's values are not limits: an unset container is not limited.
        assert_eq!(
            planned_resources(&node(None, None), &image),
            NodeResources::default()
        );
    }

    #[test]
    fn container_run_010_a_juniper_evolved_containers_limit_is_its_memory_plus_2048_mb() {
        let image = ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container))
            .unwrap()
            .node_config();
        let defaults = planned_resources(&node(None, None), &image);
        assert_eq!(defaults.memory, Some(image.memory + 2048));
        assert_eq!(defaults.cpu_count, None);
        let sized = planned_resources(&node(Some(8), Some(16384)), &image);
        assert_eq!(sized.memory, Some(16384 + 2048));
        assert_eq!(sized.cpu_count, None);
    }

    #[test]
    fn container_run_011_a_cisco_iol_containers_limit_is_its_memory_plus_128_mb() {
        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            let image = ModelKind::default_for(model).node_config();
            let defaults = planned_resources(&node(None, None), &image);
            assert_eq!(defaults.memory, Some(image.memory + 128), "{model}");
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

    /// A database with users alice and bob, and two built labs: `lab-a` (`a00000a1`) of alice,
    /// whose node `n1` holds vCPUs 2 and 3 and thread CPU 8, and `lab-b` (`b00000b1`) of bob,
    /// whose node `n1` holds nothing.
    async fn server() -> db::Database {
        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        for (index, (username, lab_id, name)) in
            [("alice", "a00000a1", "lab-a"), ("bob", "b00000b1", "lab-b")]
                .into_iter()
                .enumerate()
        {
            let user = db::create_user(&db, username.to_string(), "TestPass123!", false, vec![])
                .await
                .unwrap();
            create_lab(&db, &user, lab_id, name, index).await;
            db::update_lab_phase_from(&db, lab_id, LabPhase::Creating, LabPhase::Ready)
                .await
                .unwrap()
                .unwrap();
        }
        claim(&db, "a00000a1", pins("n1", &[2, 3], &[8])).await;
        db
    }

    async fn create_lab(db: &db::Database, user: &DbUser, lab_id: &str, name: &str, index: usize) {
        let networks = db::LabNetworks {
            management_network: format!("172.31.{index}.0/24"),
            gateway_ipv4: format!("172.31.{index}.1"),
            router_ipv4: format!("172.31.{index}.2"),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        };
        db::create_lab(db, name, lab_id, user, &networks, &LabDetails::default())
            .await
            .unwrap();
    }

    fn lab_record(lab: &DbLab) -> RecordId {
        db::get_lab_id(lab).unwrap()
    }

    /// Claim `node`'s pins in lab `lab_id`, as admission does.
    async fn claim(db: &db::Database, lab_id: &str, node: NodeCpuPins) {
        let lab = db::get_lab(db, lab_id).await.unwrap();
        db::claim_cpu_pins(db, &lab_record(&lab), &[node])
            .await
            .unwrap();
    }

    /// Keep only `node`'s pins in lab `lab_id`, as a redeploy does when it ends.
    async fn retain(db: &db::Database, lab_id: &str, node: NodeCpuPins) {
        let lab = db::get_lab(db, lab_id).await.unwrap();
        db::retain_cpu_pins(db, &lab_record(&lab), &node)
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
    async fn a_held_nodes_vcpu_cpus_are_refused_to_another_lab() {
        let db = server().await;
        let (message, conflict) = refusal(
            check_cpu_pins_free(
                &db,
                &ctx("bob", false),
                "b00000b1",
                &[pins("n1", &[3, 4], &[])],
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
        let (_, conflict) =
            refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[8], &[])]).await);
        assert_eq!(
            (conflict.requested_role, conflict.held_role),
            (CpuPinRole::Vcpu, CpuPinRole::Thread)
        );
        let (_, conflict) =
            refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[5], &[2, 3])]).await);
        assert_eq!(conflict.host_cpus, vec![2, 3]);
        assert_eq!(
            (conflict.requested_role, conflict.held_role),
            (CpuPinRole::Thread, CpuPinRole::Vcpu)
        );
        let warnings = check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[5], &[8])])
            .await
            .unwrap();
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[tokio::test]
    async fn access_control_017_admins_and_the_owner_see_the_holder_others_see_another_lab() {
        let db = server().await;
        let request = [pins("n2", &[2], &[])];
        let other =
            refusal(check_cpu_pins_free(&db, &ctx("bob", false), "b00000b1", &request).await).0;
        assert!(other.contains("of a node of another lab."), "{other}");
        assert!(
            !other.contains("lab-a") && !other.contains("a00000a1"),
            "{other}"
        );

        let holder = "the vCPUs of node 'n1' of lab 'lab-a' (a00000a1).";
        let admin =
            refusal(check_cpu_pins_free(&db, &ctx("carol", true), "b00000b1", &request).await).0;
        assert!(admin.contains(holder), "{admin}");
        // Alice owns the holding lab, and asks for its CPUs for another of her labs.
        let owner =
            refusal(check_cpu_pins_free(&db, &ctx("alice", false), "c00000c1", &request).await).0;
        assert!(owner.contains(holder), "{owner}");
    }

    #[tokio::test]
    async fn a_node_of_a_lab_being_created_holds_its_pins_and_is_named_so() {
        let db = server().await;
        let carol = db::create_user(&db, "carol".to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        create_lab(&db, &carol, "c00000c1", "lab-c", 2).await;
        claim(&db, "c00000c1", pins("n1", &[6], &[])).await;
        let (message, conflict) = refusal(
            check_cpu_pins_free(&db, &ctx("dave", true), "d00000d1", &[pins("x", &[6], &[])]).await,
        );
        assert_eq!(conflict.host_cpus, vec![6]);
        assert!(
            message.contains(
                "node 'n1' of lab 'lab-c' (c00000c1), which another operation is starting."
            ),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_requested_node_does_not_overlap_its_own_pins() {
        let db = server().await;
        let warnings = check_cpu_pins_free(
            &db,
            &ctx("alice", false),
            "a00000a1",
            &[pins("n1", &[8], &[2, 3])],
        )
        .await
        .unwrap();
        assert_eq!(warnings, Vec::<String>::new());
    }

    /// The redeploy of lab-a's `n1` from vCPUs 2 and 3 to 4 and 5, step by step: what bob's lab
    /// may pin at each point, as the table's rows then are.
    #[tokio::test]
    async fn a_redeploying_node_holds_its_old_and_new_pins_until_it_keeps_one_set() {
        let db = server().await;
        let bob = ctx("bob", false);
        let old = pins("n1", &[2, 3], &[8]);
        let new = pins("n1", &[4, 5], &[8]);

        // Admitted: its new pins are claimed beside the old ones, and both are held.
        claim(&db, "a00000a1", new.clone()).await;
        for wanted in [&old, &new] {
            refusal(check_cpu_pins_free(&db, &bob, "b00000b1", slice::from_ref(wanted)).await);
        }

        // It failed before its old domain was gone: it keeps the old ones, the new are free.
        retain(&db, "a00000a1", old.clone()).await;
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", slice::from_ref(&old)).await);
        check_cpu_pins_free(&db, &bob, "b00000b1", slice::from_ref(&new))
            .await
            .unwrap();

        // Again, and it got past teardown: it keeps the new ones, the old are free.
        claim(&db, "a00000a1", new.clone()).await;
        retain(&db, "a00000a1", new.clone()).await;
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &[new]).await);
        check_cpu_pins_free(&db, &bob, "b00000b1", &[pins("n1", &[2, 3], &[])])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn destroying_a_lab_lets_its_pins_go() {
        let db = server().await;
        let bob = ctx("bob", false);
        let wanted = [pins("n1", &[2, 3], &[])];
        refusal(check_cpu_pins_free(&db, &bob, "b00000b1", &wanted).await);
        assert!(
            db::delete_lab_from(&db, "a00000a1", LabPhase::Ready)
                .await
                .unwrap()
        );
        check_cpu_pins_free(&db, &bob, "b00000b1", &wanted)
            .await
            .unwrap();
    }

    fn holder(lab_id: &str, name: &str, node: NodeCpuPins) -> PinHolder {
        PinHolder {
            lab: Some(PinLab {
                lab_id: lab_id.to_string(),
                name: name.to_string(),
                owner: None,
            }),
            starting: false,
            pins: node,
        }
    }

    /// Rows that already share a CPU were copied as they were stored before the table existed;
    /// the trigger refuses any new overlap, so they are built here as holders.
    #[test]
    fn an_overlap_already_held_is_a_warning_and_a_new_one_is_refused() {
        let holders = [
            holder("a00000a1", "lab-a", pins("n1", &[2, 3], &[8])),
            holder("b00000b1", "lab-b", pins("n1", &[3], &[])),
        ];
        let warnings = check_pins_against_holders(
            &ctx("bob", false),
            "b00000b1",
            &[pins("n1", &[3], &[])],
            &holders,
        )
        .unwrap();
        assert_eq!(
            warnings,
            [
                "Host CPU 3 of node 'n1' is also pinned by a node of another lab, against the CPU \
              pinning rules. Starting the node anyway; redeploy one of the two on other host \
              CPUs to fix it"
            ]
        );
        let admin = check_pins_against_holders(
            &ctx("carol", true),
            "a00000a1",
            &[pins("n1", &[2, 3], &[8])],
            &holders,
        )
        .unwrap();
        assert_eq!(admin.len(), 1);
        assert!(
            admin[0].contains("node 'n1' of lab 'lab-b' (b00000b1)"),
            "{admin:?}"
        );

        // A redeploy that keeps the shared CPU but takes one more of the other node's is
        // refused for that one only.
        let (_, conflict) = refusal(check_pins_against_holders(
            &ctx("bob", false),
            "b00000b1",
            &[pins("n1", &[2, 3], &[])],
            &holders,
        ));
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
}
