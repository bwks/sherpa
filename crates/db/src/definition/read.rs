//! Reading a lab's definition back as the manifest it was deployed from.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use tracing::instrument;
use turso::{Connection, Row};

use shared::data::{CpuTopology, NodeKind, NodeModel, RecordId};
use topology::{
    Bridge, ContainerProvisioner, ContainerSecretFile, Link2, Manifest, ManifestImpairment, Node,
    NodeContainer, NodeUnikernel, NodeVm, Provisioner, ProvisionerFile, SshProvisioner,
    SystemdUnit, TailscaleConfig, TalosCluster, TextFile, VolumeMount,
};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, key_of};
use crate::queries::definition;
use crate::sql::{query_all, query_optional};

/// The lab-level columns of the definition.
struct LabRow {
    name: String,
    description: Option<String>,
    has_definition: bool,
    ready_timeout: Option<u64>,
    provision_concurrency: Option<u8>,
    tailscale_enabled: Option<bool>,
    tailscale_auth_key_env: Option<String>,
}

/// A node's key and its definition without its lists.
struct NodeRow {
    key: String,
    node: Node,
}

/// A row of a node's list table: the node's key and the item.
struct NodeItem<T> {
    node: String,
    item: T,
}

/// A text value in one of a node's lists.
struct NodeText {
    list: String,
    value: String,
}

/// A CPU in one of a node's lists.
struct NodeCpu {
    list: String,
    cpu: u16,
}

/// A file of the node's `provisioner`th provisioner.
struct ProvisionerFileRow {
    provisioner: usize,
    src: String,
    dst: Option<String>,
    name: Option<String>,
    mode: u32,
}

/// An env file or script of the node's `provisioner`th provisioner.
struct ProvisionerPathRow {
    provisioner: usize,
    list: String,
    path: String,
}

/// A link of the `bridge`th bridge.
struct BridgeLinkRow {
    bridge: usize,
    link: String,
}

/// A member of the `cluster`th Talos cluster.
struct TalosMemberRow {
    cluster: usize,
    role: String,
    node_name: String,
}

fn lab_row(row: &Row) -> Result<LabRow> {
    let mut cells = Cells::new(row);
    Ok(LabRow {
        name: cells.text("name")?,
        description: cells.optional_text("description")?,
        has_definition: cells.boolean("has_definition")?,
        ready_timeout: cells.optional_integer("ready_timeout")?,
        provision_concurrency: cells.optional_integer("provision_concurrency")?,
        tailscale_enabled: cells.optional_boolean("tailscale_enabled")?,
        tailscale_auth_key_env: cells.optional_text("tailscale_auth_key_env")?,
    })
}

fn link_row(row: &Row) -> Result<Link2> {
    let mut cells = Cells::new(row);
    let _position: i64 = cells.integer("position")?;
    let src = cells.text("src")?;
    let dst = cells.text("dst")?;
    let impaired = cells.boolean("impaired")?;
    let impairment = ManifestImpairment {
        delay: cells.optional_integer("delay")?,
        jitter: cells.optional_integer("jitter")?,
        loss_percent: cells.optional_real("loss_percent")?,
        reorder_percent: cells.optional_real("reorder_percent")?,
        corrupt_percent: cells.optional_real("corrupt_percent")?,
    };
    Ok(Link2 {
        src,
        dst,
        impairment: impaired.then_some(impairment),
    })
}

fn bridge_row(row: &Row) -> Result<Bridge> {
    let mut cells = Cells::new(row);
    let _position: i64 = cells.integer("position")?;
    Ok(Bridge {
        name: cells.text("name")?,
        links: Vec::new(),
    })
}

fn bridge_link_row(row: &Row) -> Result<BridgeLinkRow> {
    let mut cells = Cells::new(row);
    let bridge = cells.integer("bridge")?;
    let _position: i64 = cells.integer("position")?;
    Ok(BridgeLinkRow {
        bridge,
        link: cells.text("link")?,
    })
}

fn talos_cluster_row(row: &Row) -> Result<TalosCluster> {
    let mut cells = Cells::new(row);
    let _position: i64 = cells.integer("position")?;
    Ok(TalosCluster {
        name: cells.text("name")?,
        controllers: Vec::new(),
        workers: Vec::new(),
        cni: cells.enumeration("cni")?,
        oidc: cells.optional_text("oidc")?,
        vault: cells.optional_text("vault")?,
    })
}

fn talos_member_row(row: &Row) -> Result<TalosMemberRow> {
    let mut cells = Cells::new(row);
    let cluster = cells.integer("cluster")?;
    let role = cells.text("role")?;
    let _position: i64 = cells.integer("position")?;
    Ok(TalosMemberRow {
        cluster,
        role,
        node_name: cells.text("node_name")?,
    })
}

fn node_row(row: &Row) -> Result<NodeRow> {
    let mut cells = Cells::new(row);
    let key = cells.text("id")?;
    let name = cells.text("name")?;
    let model: NodeModel = cells.enumeration("model")?;
    // The node's kind is its image's. The definition holds it only when it is not the model's
    // default, so a node keeps the kind it was deployed as without a `kind` it never set. A
    // `generic` node has no default kind, so it always holds it.
    let kind: NodeKind = cells.enumeration("kind")?;
    let kind = (Some(kind) != model.default_kind()).then_some(kind);
    let image = cells.optional_text("manifest_image")?;
    let version = cells.optional_text("version")?;
    let memory = cells.optional_integer("manifest_memory")?;
    let mut node = Node {
        name,
        model,
        kind,
        image,
        version,
        memory,
        ..Node::default()
    };
    node.cpu.count = cells.optional_integer("manifest_cpu_count")?;
    node.cpu.mode = cells.optional_enumeration("cpu_mode")?;
    let sockets = cells.optional_integer("cpu_sockets")?;
    let cores = cells.optional_integer("cpu_cores")?;
    let threads = cells.optional_integer("cpu_threads")?;
    node.cpu.topology = match (sockets, cores, threads) {
        (Some(sockets), Some(cores), Some(threads)) => Some(CpuTopology {
            sockets,
            cores,
            threads,
        }),
        (None, None, None) => None,
        _ => bail!("Database fields cpu_sockets, cpu_cores and cpu_threads are not all set"),
    };
    node.numa.node = cells.optional_integer("numa_node")?;
    node.numa.memory_mode = cells.optional_enumeration("numa_memory_mode")?;
    node.boot_disk.size = cells.optional_integer("boot_disk_size")?;
    node.boot_disk.clone_method = cells.optional_enumeration("boot_disk_clone_method")?;
    node.boot_disk.cache = cells.optional_enumeration("boot_disk_cache")?;
    node.boot_disk.io = cells.optional_enumeration("boot_disk_io")?;
    node.interfaces.data_count = cells.optional_integer("interfaces_data_count")?;
    node.interfaces.queues = cells.optional_integer("interfaces_queues")?;
    node.ztp.method = cells.optional_enumeration("ztp_method")?;
    node.ztp.config = cells.optional_text("ztp_config")?;
    node.ready_check.port = cells.optional_integer("manifest_ready_port")?;
    node.ready_check.skip = cells.optional_boolean("manifest_skip_ready_check")?;
    node.vm = cells
        .optional_enumeration("vm_watchdog")?
        .map(|watchdog| NodeVm {
            watchdog: Some(watchdog),
        });
    let container = NodeContainer {
        user: cells.optional_text("container_user")?,
        privileged: cells.optional_boolean("container_privileged")?,
        shm_size: cells.optional_integer("container_shm_size")?,
        platform: cells.optional_enumeration("container_platform")?,
        ..NodeContainer::default()
    };
    node.container = (container != NodeContainer::default()).then_some(container);
    node.unikernel = cells
        .optional_text("unikernel_cmdline")?
        .map(|cmdline| NodeUnikernel {
            cmdline: Some(cmdline),
        });
    Ok(NodeRow { key, node })
}

fn node_text_row(row: &Row) -> Result<NodeItem<NodeText>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let list = cells.text("list")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: NodeText {
            list,
            value: cells.text("value")?,
        },
    })
}

fn node_cpu_row(row: &Row) -> Result<NodeItem<NodeCpu>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let list = cells.text("list")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: NodeCpu {
            list,
            cpu: cells.integer("cpu")?,
        },
    })
}

fn node_volume_row(row: &Row) -> Result<NodeItem<VolumeMount>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: VolumeMount {
            src: cells.text("src")?,
            dst: cells.text("dst")?,
        },
    })
}

fn node_text_file_row(row: &Row) -> Result<NodeItem<TextFile>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: TextFile {
            src: cells.text("src")?,
            dst: cells.text("dst")?,
            user: cells.text("user")?,
            group: cells.text("group")?,
            permissions: cells.integer("permissions")?,
        },
    })
}

fn node_systemd_unit_row(row: &Row) -> Result<NodeItem<SystemdUnit>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: SystemdUnit {
            name: cells.text("name")?,
            source: cells.text("source")?,
            enabled: cells.boolean("enabled")?,
        },
    })
}

fn node_provisioner_row(row: &Row) -> Result<NodeItem<Provisioner>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let _position: i64 = cells.integer("position")?;
    let item = match cells.text("type")?.as_str() {
        "ssh" => Provisioner::Ssh(SshProvisioner::default()),
        "container" => Provisioner::Container(ContainerProvisioner::default()),
        other => bail!("Database field type is not a provisioner type: {other}"),
    };
    Ok(NodeItem { node, item })
}

fn node_provisioner_file_row(row: &Row) -> Result<NodeItem<ProvisionerFileRow>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let provisioner = cells.integer("provisioner")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: ProvisionerFileRow {
            provisioner,
            src: cells.text("src")?,
            dst: cells.optional_text("dst")?,
            name: cells.optional_text("name")?,
            mode: cells.integer("mode")?,
        },
    })
}

fn node_provisioner_path_row(row: &Row) -> Result<NodeItem<ProvisionerPathRow>> {
    let mut cells = Cells::new(row);
    let node = cells.text("node")?;
    let provisioner = cells.integer("provisioner")?;
    let list = cells.text("list")?;
    let _position: i64 = cells.integer("position")?;
    Ok(NodeItem {
        node,
        item: ProvisionerPathRow {
            provisioner,
            list,
            path: cells.text("path")?,
        },
    })
}

/// Append `item` to the optional list `list`.
fn push<T>(list: &mut Option<Vec<T>>, item: T) {
    list.get_or_insert_with(Vec::new).push(item);
}

/// A list as the manifest has it: absent when empty.
fn non_empty<T>(list: Vec<T>) -> Option<Vec<T>> {
    (!list.is_empty()).then_some(list)
}

/// The nodes of a definition, and where each node's key is among them.
struct Nodes {
    nodes: Vec<Node>,
    by_key: HashMap<String, usize>,
}

impl Nodes {
    /// The node the row of a list table belongs to.
    fn node(&mut self, key: &str) -> Result<&mut Node> {
        let index = *self
            .by_key
            .get(key)
            .ok_or_else(|| anyhow!("A definition row belongs to node {key}, not in the lab"))?;
        self.nodes
            .get_mut(index)
            .ok_or_else(|| anyhow!("Node {key} is missing from the definition"))
    }
}

/// The lab `lab_id`'s definition as the manifest it was deployed from, or `None` when the lab
/// has none (a lab stored by a release before 0.6.4 that was never imported).
///
/// The manifest is the one stored, as the user wrote it, field for field, but for one rule: an
/// empty list or an empty `vm`, `container` or `unikernel` table is not stored, and reads back
/// as absent. It holds no file contents: every `*_data` field is absent.
///
/// # Errors
///
/// Fails when the lab does not exist, or when a stored value is not one this build can read.
#[instrument(skip(db), fields(%lab_id))]
pub async fn read_lab_definition(db: &Database, lab_id: &RecordId) -> Result<Option<Manifest>> {
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let conn = db.reader().await?;
    let lab = query_optional(
        &conn,
        definition::FIND_LAB.bind([lab_key.as_str().into()]),
        lab_row,
    )
    .await?
    .ok_or_else(|| anyhow!("Lab not found: {lab_id}"))?;
    if !lab.has_definition {
        return Ok(None);
    }

    let links = query_all(
        &conn,
        definition::LIST_LINKS.bind([lab_key.as_str().into()]),
        link_row,
    )
    .await?;
    let bridges = read_bridges(&conn, &lab_key).await?;
    let clusters = read_talos_clusters(&conn, &lab_key).await?;

    let rows = query_all(
        &conn,
        definition::LIST_NODES.bind([lab_key.as_str().into()]),
        node_row,
    )
    .await?;
    let mut nodes = Nodes {
        by_key: rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.key.clone(), index))
            .collect(),
        nodes: rows.into_iter().map(|row| row.node).collect(),
    };
    read_node_lists(&conn, &lab_key, &mut nodes).await?;
    read_node_files(&conn, &lab_key, &mut nodes).await?;
    read_provisioners(&conn, &lab_key, &mut nodes).await?;

    Ok(Some(Manifest {
        name: lab.name,
        description: lab.description,
        tailscale: lab.tailscale_enabled.map(|enabled| TailscaleConfig {
            enabled,
            auth_key_env: lab.tailscale_auth_key_env,
        }),
        ready_timeout: lab.ready_timeout,
        provision_concurrency: lab.provision_concurrency,
        talos_clusters: non_empty(clusters),
        nodes: nodes.nodes,
        links: non_empty(links),
        bridges: non_empty(bridges),
    }))
}

/// Read the lab's bridges and the ends each names.
async fn read_bridges(conn: &Connection, lab_key: &str) -> Result<Vec<Bridge>> {
    let mut bridges = query_all(
        conn,
        definition::LIST_BRIDGES.bind([lab_key.into()]),
        bridge_row,
    )
    .await?;
    for link in query_all(
        conn,
        definition::LIST_BRIDGE_LINKS.bind([lab_key.into()]),
        bridge_link_row,
    )
    .await?
    {
        bridges
            .get_mut(link.bridge)
            .ok_or_else(|| {
                anyhow!(
                    "A bridge link belongs to bridge {}, not stored",
                    link.bridge
                )
            })?
            .links
            .push(link.link);
    }
    Ok(bridges)
}

/// Read the lab's Talos clusters and their members.
async fn read_talos_clusters(conn: &Connection, lab_key: &str) -> Result<Vec<TalosCluster>> {
    let mut clusters = query_all(
        conn,
        definition::LIST_TALOS_CLUSTERS.bind([lab_key.into()]),
        talos_cluster_row,
    )
    .await?;
    for member in query_all(
        conn,
        definition::LIST_TALOS_MEMBERS.bind([lab_key.into()]),
        talos_member_row,
    )
    .await?
    {
        let cluster = clusters.get_mut(member.cluster).ok_or_else(|| {
            anyhow!(
                "A Talos member belongs to cluster {}, not stored",
                member.cluster
            )
        })?;
        match member.role.as_str() {
            "controller" => cluster.controllers.push(member.node_name),
            "worker" => cluster.workers.push(member.node_name),
            other => bail!("Database field role is not a Talos cluster role: {other}"),
        }
    }
    Ok(clusters)
}

/// Read the nodes' lists of CPUs and text values, and their container volumes.
async fn read_node_lists(conn: &Connection, lab_key: &str, nodes: &mut Nodes) -> Result<()> {
    for row in query_all(
        conn,
        definition::LIST_NODE_CPUS.bind([lab_key.into()]),
        node_cpu_row,
    )
    .await?
    {
        let node = nodes.node(&row.node)?;
        let list = match row.item.list.as_str() {
            "pinning" => &mut node.cpu.pinning,
            "emulator_pinning" => &mut node.cpu.emulator_pinning,
            "io_thread" => &mut node.boot_disk.io_thread_cpus,
            other => bail!("Database field list is not a CPU list: {other}"),
        };
        push(list, row.item.cpu);
    }
    for row in query_all(
        conn,
        definition::LIST_NODE_TEXTS.bind([lab_key.into()]),
        node_text_row,
    )
    .await?
    {
        let node = nodes.node(&row.node)?;
        let value = row.item.value;
        match row.item.list.as_str() {
            "container_command" => push(
                &mut node
                    .container
                    .get_or_insert_with(NodeContainer::default)
                    .commands,
                value,
            ),
            "environment_variable" => push(&mut node.environment_variables, value),
            "startup_script" => push(&mut node.startup_scripts, value),
            other => bail!("Database field list is not a text list: {other}"),
        }
    }
    for row in query_all(
        conn,
        definition::LIST_NODE_VOLUMES.bind([lab_key.into()]),
        node_volume_row,
    )
    .await?
    {
        let node = nodes.node(&row.node)?;
        push(
            &mut node
                .container
                .get_or_insert_with(NodeContainer::default)
                .volumes,
            row.item,
        );
    }
    Ok(())
}

/// Read the nodes' `text_files` and `systemd_units`.
async fn read_node_files(conn: &Connection, lab_key: &str, nodes: &mut Nodes) -> Result<()> {
    for row in query_all(
        conn,
        definition::LIST_NODE_TEXT_FILES.bind([lab_key.into()]),
        node_text_file_row,
    )
    .await?
    {
        push(&mut nodes.node(&row.node)?.text_files, row.item);
    }
    for row in query_all(
        conn,
        definition::LIST_NODE_SYSTEMD_UNITS.bind([lab_key.into()]),
        node_systemd_unit_row,
    )
    .await?
    {
        push(&mut nodes.node(&row.node)?.systemd_units, row.item);
    }
    Ok(())
}

/// Read the nodes' provisioners, their files and an ssh provisioner's env files and scripts.
async fn read_provisioners(conn: &Connection, lab_key: &str, nodes: &mut Nodes) -> Result<()> {
    for row in query_all(
        conn,
        definition::LIST_NODE_PROVISIONERS.bind([lab_key.into()]),
        node_provisioner_row,
    )
    .await?
    {
        push(&mut nodes.node(&row.node)?.provisioners, row.item);
    }
    for row in query_all(
        conn,
        definition::LIST_NODE_PROVISIONER_FILES.bind([lab_key.into()]),
        node_provisioner_file_row,
    )
    .await?
    {
        let file = row.item;
        let provisioner = provisioner_at(nodes.node(&row.node)?, file.provisioner)?;
        match (provisioner, file.dst, file.name) {
            (Provisioner::Ssh(ssh), Some(dst), None) => ssh.files.push(ProvisionerFile {
                src: file.src,
                dst,
                mode: file.mode,
            }),
            (Provisioner::Container(container), None, Some(name)) => {
                container.files.push(ContainerSecretFile {
                    src: file.src,
                    name,
                    mode: file.mode,
                });
            }
            _ => bail!("A provisioner file row does not fit its provisioner's type"),
        }
    }
    for row in query_all(
        conn,
        definition::LIST_NODE_PROVISIONER_PATHS.bind([lab_key.into()]),
        node_provisioner_path_row,
    )
    .await?
    {
        let path = row.item;
        let Provisioner::Ssh(ssh) = provisioner_at(nodes.node(&row.node)?, path.provisioner)?
        else {
            bail!("A provisioner path row belongs to a container provisioner");
        };
        match path.list.as_str() {
            "env_file" => ssh.env_files.push(path.path),
            "script" => ssh.scripts.push(path.path),
            other => bail!("Database field list is not a provisioner path list: {other}"),
        }
    }
    Ok(())
}

/// The node's `position`th provisioner.
fn provisioner_at(node: &mut Node, position: usize) -> Result<&mut Provisioner> {
    node.provisioners
        .as_mut()
        .and_then(|provisioners| provisioners.get_mut(position))
        .ok_or_else(|| anyhow!("A provisioner row refers to provisioner {position}, not stored"))
}
