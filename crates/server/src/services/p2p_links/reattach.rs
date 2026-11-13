//! Re-attaching a node's P2p links after its end was made again: the one transition every
//! event that makes an end again goes through (a VM or unikernel cold boot, a container start,
//! a redeploy).

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::instrument;

use super::host::LinkHost;
use super::transition::{End, EndAt, LinkStep, Side, step};
use super::veths::{ContainerVeth, node_links};
use crate::daemon::link_states::LinkStates;
use crate::services::progress::ProgressSender;

use shared::data::{DbLink, DbNode, LinkImpairment, LinkReport, LinkState, RecordId, StatusKind};

/// The lab's links and node names, as the database has them: what re-attach works from.
pub(crate) struct LabLinks {
    links: Vec<DbLink>,
    names: HashMap<RecordId, String>,
}

/// How the node's ends were made again.
pub(crate) enum RecreatedEnd {
    /// libvirt made the node's taps when it started its domain: a VM or unikernel cold boot,
    /// or a VM redeploy.
    Domain,
    /// The container started in a new network namespace, with none of its data interfaces:
    /// its veths, named `container`'s, are made first.
    Container {
        container: String,
        veths: Vec<ContainerVeth>,
    },
    /// The node's ends could not be planned, and why. Each of its links fails.
    Unplanned(String),
}

/// What re-attaching left one link in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkOutcome {
    /// The link's index in its lab.
    pub(crate) index: u16,
    pub(crate) report: LinkReport,
}

/// A link's two ends, with their indexes and impairments: what attaching it sets.
pub(crate) struct LinkWiring<'a> {
    pub(crate) tap_a: &'a str,
    pub(crate) tap_b: &'a str,
    pub(crate) ifindex_a: u32,
    pub(crate) ifindex_b: u32,
    pub(crate) a_to_b: &'a LinkImpairment,
    pub(crate) b_to_a: &'a LinkImpairment,
}

impl LabLinks {
    /// The lab's links and the names of its nodes.
    pub(crate) fn new(links: Vec<DbLink>, nodes: &[DbNode]) -> Self {
        let names = nodes
            .iter()
            .filter_map(|node| node.id.clone().map(|id| (id, node.name.clone())))
            .collect();
        Self { links, names }
    }

    /// Load the lab's links and node names.
    #[instrument(skip(db), fields(%lab), level = "debug")]
    pub(crate) async fn load(db: &db::Database, lab: &RecordId) -> Result<Self> {
        let links = db::list_links_by_lab(db, lab.clone())
            .await
            .context("Failed to list the lab's links")?;
        let nodes = db::list_nodes_by_lab(db, lab.clone())
            .await
            .context("Failed to list the lab's nodes")?;
        Ok(Self::new(links, &nodes))
    }

    /// The lab's links.
    pub(crate) fn links(&self) -> &[DbLink] {
        &self.links
    }

    /// The name of node `id`, or its record ID when it has none.
    pub(crate) fn name(&self, id: &RecordId) -> String {
        self.names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    /// `link` as the user knows it: `node::interface <-> node::interface`.
    pub(crate) fn label(&self, link: &DbLink) -> String {
        format!(
            "{}::{} <-> {}::{}",
            self.name(&link.node_a),
            link.int_a,
            self.name(&link.node_b),
            link.int_b
        )
    }
}

/// Re-attach every link of `node`, whose ends were made again as `end` says, and report what
/// each link was left in, in the order of the links' indexes.
///
/// Each link is decided on its own (see `transition`), so one that fails does not stop the
/// others. A link whose peer is down is left for the peer's own re-attach. Nothing here fails
/// the node: the caller reports the links with the node's result.
#[instrument(skip_all, fields(%node), level = "debug")]
pub(crate) async fn reattach_node_links(
    host: &impl LinkHost,
    lab: &LabLinks,
    node: &RecordId,
    end: RecreatedEnd,
) -> Vec<LinkOutcome> {
    let mut links = node_links(&lab.links, node);
    links.sort_by_key(|link| link.index);
    let broken = match end {
        RecreatedEnd::Domain => HashMap::new(),
        RecreatedEnd::Container { container, veths } => {
            make_container_ends(host, &container, &veths).await
        }
        RecreatedEnd::Unplanned(reason) => links
            .iter()
            .flat_map(|link| [(&link.node_a, &link.tap_a), (&link.node_b, &link.tap_b)])
            .filter(|(end_node, _)| *end_node == node)
            .map(|(_, tap)| (tap.clone(), reason.clone()))
            .collect(),
    };

    let mut reports = vec![];
    for link in links {
        let side = match (link.node_a == *node, link.node_b == *node) {
            (true, true) => Side::Both,
            (true, false) => Side::A,
            _ => Side::B,
        };
        let peer = match side {
            Side::B => &link.node_a,
            Side::A | Side::Both => &link.node_b,
        };
        let a = EndAt {
            iface: &link.tap_a,
            end: look_up(host, &link.tap_a, &broken).await,
        };
        let b = EndAt {
            iface: &link.tap_b,
            end: look_up(host, &link.tap_b, &broken).await,
        };
        let state = match step(side, &a, &b, &lab.name(peer)) {
            LinkStep::Attach {
                ifindex_a,
                ifindex_b,
            } => {
                let wiring = LinkWiring {
                    tap_a: &link.tap_a,
                    tap_b: &link.tap_b,
                    ifindex_a,
                    ifindex_b,
                    a_to_b: &link.impairment_a_to_b,
                    b_to_a: &link.impairment_b_to_a,
                };
                match attach_link(host, &wiring).await {
                    Ok(()) => LinkState::Attached,
                    Err(error) => LinkState::Failed {
                        reason: format!("{error:#}"),
                    },
                }
            }
            LinkStep::Leave(state) => state,
        };
        reports.push(LinkOutcome {
            index: link.index,
            report: LinkReport {
                link: lab.label(link),
                state,
            },
        });
    }
    reports
}

/// Make a started container's veths in its network namespace; with none to make, the container
/// is not looked up. Returns the host ends that could
/// not be made, each with why: the others are made whatever happens to one.
#[instrument(skip(host, veths), level = "debug")]
async fn make_container_ends(
    host: &impl LinkHost,
    container: &str,
    veths: &[ContainerVeth],
) -> HashMap<String, String> {
    if veths.is_empty() {
        return HashMap::new();
    }
    let pid = match host.container_pid(container).await {
        Ok(pid) => pid,
        Err(error) => {
            let reason =
                format!("cannot find container {container}'s network namespace: {error:#}");
            return veths
                .iter()
                .map(|veth| (veth.host_veth.clone(), reason.clone()))
                .collect();
        }
    };
    let mut broken = HashMap::new();
    for veth in veths {
        if let Err(error) = host.create_veth(veth, pid).await {
            tracing::warn!(container = %container, host_veth = %veth.host_veth, error = %error, "Failed to make a container veth");
            broken.insert(veth.host_veth.clone(), format!("{error:#}"));
        }
    }
    broken
}

/// What the kernel shows of end `iface`, or why it is broken when it could not be made.
async fn look_up(host: &impl LinkHost, iface: &str, broken: &HashMap<String, String>) -> End {
    if let Some(reason) = broken.get(iface) {
        return End::Broken(reason.clone());
    }
    match host.ifindex(iface).await {
        Ok(Some(ifindex)) => End::Present(ifindex),
        Ok(None) => End::Absent,
        Err(error) => End::Broken(format!("failed to look up {iface}: {error:#}")),
    }
}

/// Point each end of a link at the other, and set each direction's impairment on the end it
/// leaves through: traffic from A to B leaves through B's end.
///
/// # Errors
///
/// Fails at the first host operation that fails.
#[instrument(skip_all, fields(tap_a = %wiring.tap_a, tap_b = %wiring.tap_b), level = "debug")]
pub(crate) async fn attach_link(host: &impl LinkHost, wiring: &LinkWiring<'_>) -> Result<()> {
    host.attach_redirect(wiring.tap_a, wiring.ifindex_b)?;
    host.attach_redirect(wiring.tap_b, wiring.ifindex_a)?;
    host.sync_netem(wiring.ifindex_b, wiring.a_to_b).await?;
    host.sync_netem(wiring.ifindex_a, wiring.b_to_a).await?;
    tracing::info!(
        tap_a = %wiring.tap_a,
        tap_b = %wiring.tap_b,
        ifindex_a = wiring.ifindex_a,
        ifindex_b = wiring.ifindex_b,
        "Attached eBPF P2p redirect"
    );
    Ok(())
}

/// Record what each of `node_name`'s links in lab `lab_id` was left in, for `inspect`, and
/// tell the user about each that is not attached, and log it. Returns the links' reports.
pub(crate) fn report_links(
    progress: Option<&ProgressSender>,
    link_states: &LinkStates,
    lab_id: &str,
    node_name: &str,
    outcomes: Vec<LinkOutcome>,
) -> Vec<LinkReport> {
    let mut reports = Vec::with_capacity(outcomes.len());
    for LinkOutcome { index, report } in outcomes {
        link_states.record(lab_id, index, report.state.clone());
        match &report.state {
            LinkState::Attached => {}
            LinkState::PeerDown { .. } => {
                tracing::info!(node = %node_name, link = %report.link, "P2p link waits for its peer");
            }
            LinkState::Failed { reason } => {
                tracing::warn!(node = %node_name, link = %report.link, %reason, "P2p link not re-attached");
            }
        }
        if report.state != LinkState::Attached
            && let Some(progress) = progress
        {
            let _ = progress.send_status(format!("Node {node_name}: {report}"), StatusKind::Info);
        }
        reports.push(report);
    }
    reports
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command;

    use bollard::Docker;

    use crate::services::p2p_links::KernelHost;
    use crate::services::p2p_links::fake::FakeHost;
    use shared::data::{BridgeKind, NodeModel, NodeState};

    fn node(name: &str) -> DbNode {
        DbNode {
            id: Some(RecordId::new("node", name)),
            name: name.to_string(),
            image: RecordId::new("node_image", "img"),
            index: 1,
            lab: RecordId::new("lab", "lab"),
            mgmt_ipv4: None,
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    fn link(index: u16, node_a: &str, node_b: &str) -> DbLink {
        DbLink {
            id: None,
            index,
            kind: BridgeKind::P2p,
            node_a: RecordId::new("node", node_a),
            node_b: RecordId::new("node", node_b),
            int_a: format!("eth{}", index + 1),
            int_b: "eth1".to_string(),
            lab: RecordId::new("lab", "lab"),
            tap_a: format!("tpa{index}"),
            tap_b: format!("tpb{index}"),
            impairment_a_to_b: LinkImpairment::default(),
            impairment_b_to_a: LinkImpairment::default(),
        }
    }

    /// vm1 with a link to vm2 (0), ctr1 (1) and uk1 (2), every end made and attached.
    fn lab() -> (FakeHost, LabLinks) {
        let nodes = ["vm1", "vm2", "ctr1", "uk1"].map(node);
        let links = vec![
            link(0, "vm1", "vm2"),
            link(1, "vm1", "ctr1"),
            link(2, "vm1", "uk1"),
        ];
        let host = FakeHost::default();
        host.make(&["tpa0", "tpb0", "tpa1", "tpb1", "tpa2", "tpb2"]);
        (host, LabLinks::new(links, &nodes))
    }

    fn id(name: &str) -> RecordId {
        RecordId::new("node", name)
    }

    fn reports_of(outcomes: Vec<LinkOutcome>) -> Vec<LinkReport> {
        outcomes.into_iter().map(|outcome| outcome.report).collect()
    }

    fn states(reports: &[LinkReport]) -> Vec<(&str, &LinkState)> {
        reports
            .iter()
            .map(|report| (report.link.as_str(), &report.state))
            .collect()
    }

    fn peer_down(peer: &str) -> LinkState {
        LinkState::PeerDown {
            peer: peer.to_string(),
        }
    }

    #[tokio::test]
    async fn a_cold_booted_node_has_every_link_attached_to_its_new_ends() {
        let (host, lab) = lab();
        // A cold boot gives vm1 new taps: each peer's redirect is stale.
        host.make(&["tpa0", "tpa1", "tpa2"]);
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert!(
            reports.iter().all(|r| r.state == LinkState::Attached),
            "{reports:?}"
        );
        for (a, b) in [("tpa0", "tpb0"), ("tpa1", "tpb1"), ("tpa2", "tpb2")] {
            assert!(host.wired(a, b), "{a} <-> {b}");
        }
    }

    #[tokio::test]
    async fn a_peer_that_is_down_does_not_stop_the_other_links() {
        // M25.1: vm2, the first link's peer, is shut off; vm1 alone is resumed.
        let (host, lab) = lab();
        host.remove(&["tpb0"]);
        host.make(&["tpa0", "tpa1", "tpa2"]);
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(
            states(&reports),
            vec![
                ("vm1::eth1 <-> vm2::eth1", &peer_down("vm2")),
                ("vm1::eth2 <-> ctr1::eth1", &LinkState::Attached),
                ("vm1::eth3 <-> uk1::eth1", &LinkState::Attached),
            ]
        );
        assert!(host.wired("tpa1", "tpb1"));
        assert!(host.wired("tpa2", "tpb2"));
    }

    #[tokio::test]
    async fn a_link_that_fails_does_not_stop_the_other_links() {
        // M25.1: the first link fails in the kernel; the later ones are still attached.
        let (mut host, lab) = lab();
        host.failing_redirects.insert("tpb0".to_string());
        host.failing_lookups.insert("tpb1".to_string());
        host.make(&["tpa0", "tpa1", "tpa2"]);
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert!(reports[0].failed(), "{reports:?}");
        assert!(reports[1].failed(), "{reports:?}");
        assert_eq!(reports[2].state, LinkState::Attached);
        assert!(host.wired("tpa2", "tpb2"));
    }

    #[tokio::test]
    async fn a_peer_that_comes_back_attaches_both_directions() {
        // M9.2: vm1 is rebuilt while vm2 is shut off, then vm2 is resumed.
        let (host, lab) = lab();
        host.remove(&["tpb0"]);
        host.make(&["tpa0"]);
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(reports[0].state, peer_down("vm2"));
        assert!(!host.wired("tpa0", "tpb0"));

        host.make(&["tpb0"]);
        let reports = reattach_node_links(&host, &lab, &id("vm2"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(
            states(&reports),
            vec![("vm1::eth1 <-> vm2::eth1", &LinkState::Attached)]
        );
        assert!(
            host.wired("tpa0", "tpb0"),
            "both directions point at the current ends"
        );
    }

    #[tokio::test]
    async fn a_node_whose_own_end_is_missing_fails_the_link_not_the_node() {
        let (host, lab) = lab();
        host.remove(&["tpa2"]);
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(
            reports[2].state,
            LinkState::Failed {
                reason: "its end tpa2 does not exist".to_string()
            }
        );
        assert!(host.wired("tpa0", "tpb0"));
        assert!(host.wired("tpa1", "tpb1"));
    }

    fn ctr1_veths() -> Vec<ContainerVeth> {
        let veth = |host_veth: &str, idx: u8, admin_down: bool| ContainerVeth {
            host_veth: host_veth.to_string(),
            host_alias: format!("alias-{host_veth}"),
            interface_idx: idx,
            node_model: NodeModel::FrrLinux,
            admin_down,
            bridge: None,
        };
        vec![veth("tpb1", 1, false), veth("cd1i2-lab", 2, true)]
    }

    #[tokio::test]
    async fn a_started_container_gets_its_veths_again_before_its_links_attach() {
        // M25.2: the container exited, so its namespace and its veths are gone.
        let (host, lab) = lab();
        host.remove(&["tpb1"]);
        let reports = reattach_node_links(
            &host,
            &lab,
            &id("ctr1"),
            RecreatedEnd::Container {
                container: "ctr1-lab".to_string(),
                veths: ctr1_veths(),
            },
        )
        .await;
        let reports = reports_of(reports);
        assert_eq!(
            states(&reports),
            vec![("vm1::eth2 <-> ctr1::eth1", &LinkState::Attached)]
        );
        assert!(host.wired("tpa1", "tpb1"));
        assert!(
            host.index("cd1i2-lab").is_some(),
            "disabled interfaces are made too"
        );
        assert_eq!(
            host.redirect("cd1i2-lab"),
            None,
            "a disabled interface has no redirect"
        );
    }

    #[tokio::test]
    async fn a_veth_that_cannot_be_made_fails_its_link() {
        let (mut host, lab) = lab();
        host.remove(&["tpb1"]);
        host.failing_veths.insert("tpb1".to_string());
        let reports = reattach_node_links(
            &host,
            &lab,
            &id("ctr1"),
            RecreatedEnd::Container {
                container: "ctr1-lab".to_string(),
                veths: ctr1_veths(),
            },
        )
        .await;
        let reports = reports_of(reports);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].failed(), "{reports:?}");
        assert!(
            host.index("cd1i2-lab").is_some(),
            "the other veths are still made"
        );
    }

    #[tokio::test]
    async fn a_container_whose_namespace_cannot_be_found_fails_its_links() {
        let (mut host, lab) = lab();
        host.remove(&["tpb1"]);
        host.no_container = true;
        let reports = reattach_node_links(
            &host,
            &lab,
            &id("ctr1"),
            RecreatedEnd::Container {
                container: "ctr1-lab".to_string(),
                veths: ctr1_veths(),
            },
        )
        .await;
        let reports = reports_of(reports);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].failed(), "{reports:?}");
    }

    #[tokio::test]
    async fn a_node_whose_ends_could_not_be_planned_fails_each_link() {
        let (host, lab) = lab();
        let reports = reattach_node_links(
            &host,
            &lab,
            &id("vm1"),
            RecreatedEnd::Unplanned("no manifest".to_string()),
        )
        .await;
        let reports = reports_of(reports);
        assert_eq!(reports.len(), 3);
        assert!(reports.iter().all(LinkReport::failed), "{reports:?}");
    }

    #[tokio::test]
    async fn re_attach_sets_each_direction_on_the_end_it_leaves_through() {
        let (host, mut lab) = lab();
        let delay = LinkImpairment {
            delay_us: 5000,
            ..LinkImpairment::default()
        };
        lab.links[0].impairment_a_to_b = delay;
        // A stale qdisc left on vm1's end, from a removal that failed (M25.3), is cleared.
        let ifindex_a = host.index("tpa0").unwrap();
        host.netem.borrow_mut().insert(ifindex_a, delay);
        let reports = reattach_node_links(&host, &lab, &id("vm2"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(reports[0].state, LinkState::Attached);
        let netem = host.netem.borrow();
        assert_eq!(netem.get(&host.index("tpb0").unwrap()), Some(&delay));
        assert_eq!(netem.get(&ifindex_a), None);
    }

    /// Whether `iface` has a TC filter on its ingress running the redirect program.
    fn has_redirect(iface: &str) -> bool {
        let output = Command::new("tc")
            .args(["filter", "show", "dev", iface, "ingress"])
            .output()
            .expect("tc runs");
        String::from_utf8_lossy(&output.stdout).contains("p2p_redirect")
    }

    #[tokio::test]
    #[ignore = "requires root: makes veths and attaches eBPF programs"]
    async fn the_kernel_re_attaches_a_link_and_waits_for_a_peer_that_is_down() {
        let (a, b) = ("it-reatt-a", "it-reatt-b");
        let make_end = |name: &'static str| async move {
            let _ = network::delete_interface(name).await;
            network::create_veth_pair(name, &format!("{name}p"), name, name)
                .await
                .unwrap();
        };
        make_end(a).await;
        make_end(b).await;
        let mut link = link(0, "vm1", "vm2");
        link.tap_a = a.to_string();
        link.tap_b = b.to_string();
        let lab = LabLinks::new(vec![link], &[node("vm1"), node("vm2")]);
        let docker = Docker::connect_with_local_defaults().unwrap();
        let host = KernelHost::new(&docker);

        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(reports[0].state, LinkState::Attached, "{reports:?}");
        assert!(has_redirect(a) && has_redirect(b));

        // vm2's end goes, as a shut-off VM's tap does: vm1's re-attach waits for it.
        network::delete_interface(b).await.unwrap();
        let reports = reattach_node_links(&host, &lab, &id("vm1"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(reports[0].state, peer_down("vm2"));

        // vm2 comes back with a new end: its own re-attach sets both directions.
        make_end(b).await;
        let reports = reattach_node_links(&host, &lab, &id("vm2"), RecreatedEnd::Domain).await;
        let reports = reports_of(reports);
        assert_eq!(reports[0].state, LinkState::Attached, "{reports:?}");
        assert!(has_redirect(a) && has_redirect(b));

        network::delete_interface(a).await.unwrap();
        network::delete_interface(b).await.unwrap();
    }
}
