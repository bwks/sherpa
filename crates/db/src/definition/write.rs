//! Writing a lab's definition: the manifest it was deployed from, as the user wrote it, in the
//! lab's and nodes' rows and the `manifest_*` tables. No file contents are written.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tracing::instrument;
use turso::{Connection, Value};

use shared::data::RecordId;
use topology::{Manifest, Node, Provisioner};

use crate::Database;
use crate::persistence::{LAB_TABLE, NODE_TABLE, enum_text, key_of, node_from_row};
use crate::queries::{definition, node as node_queries};
use crate::secret::seal;
use crate::sql::{execute, query_all};

/// Refuse a `manifest` that holds the contents of its files, which a definition never stores.
/// It must be a manifest as written ([`Manifest::definition`]).
fn refuse_file_contents(manifest: &Manifest) -> Result<()> {
    if manifest.has_file_contents() {
        bail!(
            "Refusing to store the contents of files as a lab definition: store the manifest as \
             written"
        );
    }
    Ok(())
}

/// The place of a lab's sealed `ztp_server.password`, its associated data.
pub(super) fn ztp_password_context(lab_key: &str) -> String {
    format!("lab.ztp_server_password:{lab_key}")
}

/// Store `manifest`, a manifest as the user wrote it, as the definition of the lab `lab_id`,
/// replacing any it had: the lab-level parameters, its links, bridges and Talos clusters, and
/// every node's parameters. Each node of the manifest must have its record in the lab already.
/// One transaction, so a failure leaves the definition as it was.
///
/// # Errors
///
/// Fails when the manifest holds file contents (a `*_data` field), when the lab or one of the
/// manifest's nodes has no record, when the manifest has a `ztp_server.password` and the database
/// has no key, or when a value does not fit its column.
#[instrument(skip(db, manifest), fields(%lab_id, lab_name = %manifest.name))]
pub async fn store_lab_definition(
    db: &Database,
    lab_id: &RecordId,
    manifest: &Manifest,
) -> Result<()> {
    refuse_file_contents(manifest)?;
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let password = match manifest
        .ztp_server
        .as_ref()
        .and_then(|ztp_server| ztp_server.password.as_deref())
    {
        Some(password) => Some(seal(
            db.secret_key()?,
            password.as_bytes(),
            &ztp_password_context(&lab_key),
        )?),
        None => None,
    };
    let ready_timeout = manifest
        .ready_timeout
        .map(i64::try_from)
        .transpose()
        .context("ready_timeout is too large to store")?;
    let tailscale = manifest.tailscale.as_ref();
    let ztp_server = manifest.ztp_server.as_ref();

    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the lab definition transaction")?;
    let changed = execute(
        &tx,
        definition::UPDATE_LAB.bind([
            lab_key.as_str().into(),
            ready_timeout.into(),
            manifest.provision_current_user.into(),
            manifest.provision_concurrency.into(),
            tailscale.map(|tailscale| tailscale.enabled).into(),
            tailscale
                .and_then(|tailscale| tailscale.auth_key_env.as_deref())
                .into(),
            ztp_server.map(|ztp_server| ztp_server.enable).into(),
            ztp_server
                .and_then(|ztp_server| ztp_server.username.as_deref())
                .into(),
            password.into(),
        ]),
    )
    .await?;
    if changed != 1 {
        bail!("Lab not found: {lab_id}");
    }
    write_links(&tx, &lab_key, manifest).await?;
    write_bridges(&tx, &lab_key, manifest).await?;
    write_talos_clusters(&tx, &lab_key, manifest).await?;

    let records = query_all(
        &tx,
        node_queries::LIST_BY_LAB.bind([lab_key.as_str().into()]),
        node_from_row,
    )
    .await?;
    for node in &manifest.nodes {
        let Some(id) = records
            .iter()
            .find(|record| record.name == node.name)
            .and_then(|record| record.id.as_ref())
        else {
            bail!("Node '{}' has no record in lab {lab_id}", node.name);
        };
        write_node(&tx, &key_of(id, NODE_TABLE)?, node).await?;
    }
    tx.commit()
        .await
        .context("Failed to commit the lab definition")?;
    Ok(())
}

/// Store what a redeploy of `node_name` from `manifest`, a manifest as written, changes in the
/// definition of the lab `lab_id`: the node's parameters, replacing its old ones, and the
/// lab-level settings the redeploy applies, `ready_timeout`, `provision_current_user` and
/// `provision_concurrency`. The lab's links, bridges, Talos clusters, Tailscale and ZTP server
/// settings are what `up` built, and a redeploy does not change them. One transaction.
///
/// # Errors
///
/// Fails when the manifest holds file contents, when it has no node `node_name`, when `node_id`
/// is not a node record, or when a value does not fit its column.
#[instrument(skip(db, manifest), fields(%lab_id, %node_name))]
pub async fn store_redeployed_node(
    db: &Database,
    lab_id: &RecordId,
    node_id: &RecordId,
    node_name: &str,
    manifest: &Manifest,
) -> Result<()> {
    refuse_file_contents(manifest)?;
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let node_key = key_of(node_id, NODE_TABLE)?;
    let Some(node) = manifest.nodes.iter().find(|node| node.name == node_name) else {
        bail!("Node '{node_name}' is not in the manifest");
    };
    let ready_timeout = manifest
        .ready_timeout
        .map(i64::try_from)
        .transpose()
        .context("ready_timeout is too large to store")?;

    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the node definition transaction")?;
    let changed = execute(
        &tx,
        definition::UPDATE_LAB_SETTINGS.bind([
            lab_key.as_str().into(),
            ready_timeout.into(),
            manifest.provision_current_user.into(),
            manifest.provision_concurrency.into(),
        ]),
    )
    .await?;
    if changed != 1 {
        bail!("Lab not found: {lab_id}");
    }
    write_node(&tx, &node_key, node).await?;
    tx.commit()
        .await
        .context("Failed to commit the node definition")?;
    Ok(())
}

/// An optional enum as the text its serde name is, or NULL.
fn optional_enum<T: Serialize>(value: Option<&T>, field: &str) -> Result<Value> {
    Ok(value
        .map(|value| enum_text(value, field))
        .transpose()?
        .into())
}

/// A list position.
fn position(index: usize) -> Result<Value> {
    Ok(i64::try_from(index)
        .context("A list is too long to store")?
        .into())
}

async fn write_links(conn: &Connection, lab_key: &str, manifest: &Manifest) -> Result<()> {
    execute(conn, definition::DELETE_LINKS.bind([lab_key.into()])).await?;
    for (index, link) in manifest.links.iter().flatten().enumerate() {
        let impairment = link.impairment.as_ref();
        execute(
            conn,
            definition::CREATE_LINK.bind([
                lab_key.into(),
                position(index)?,
                link.src.as_str().into(),
                link.dst.as_str().into(),
                link.p2p.into(),
                impairment.is_some().into(),
                impairment.and_then(|impairment| impairment.delay).into(),
                impairment.and_then(|impairment| impairment.jitter).into(),
                impairment
                    .and_then(|impairment| impairment.loss_percent)
                    .into(),
                impairment
                    .and_then(|impairment| impairment.reorder_percent)
                    .into(),
                impairment
                    .and_then(|impairment| impairment.corrupt_percent)
                    .into(),
            ]),
        )
        .await
        .with_context(|| format!("Failed to store link {} -- {}", link.src, link.dst))?;
    }
    Ok(())
}

async fn write_bridges(conn: &Connection, lab_key: &str, manifest: &Manifest) -> Result<()> {
    execute(conn, definition::DELETE_BRIDGES.bind([lab_key.into()])).await?;
    for (index, bridge) in manifest.bridges.iter().flatten().enumerate() {
        execute(
            conn,
            definition::CREATE_BRIDGE.bind([
                lab_key.into(),
                position(index)?,
                bridge.name.as_str().into(),
            ]),
        )
        .await
        .with_context(|| format!("Failed to store bridge {}", bridge.name))?;
        for (link_index, link) in bridge.links.iter().enumerate() {
            execute(
                conn,
                definition::CREATE_BRIDGE_LINK.bind([
                    lab_key.into(),
                    position(index)?,
                    position(link_index)?,
                    link.as_str().into(),
                ]),
            )
            .await
            .with_context(|| format!("Failed to store bridge {} link {link}", bridge.name))?;
        }
    }
    Ok(())
}

async fn write_talos_clusters(conn: &Connection, lab_key: &str, manifest: &Manifest) -> Result<()> {
    execute(
        conn,
        definition::DELETE_TALOS_CLUSTERS.bind([lab_key.into()]),
    )
    .await?;
    for (index, cluster) in manifest.talos_clusters.iter().flatten().enumerate() {
        execute(
            conn,
            definition::CREATE_TALOS_CLUSTER.bind([
                lab_key.into(),
                position(index)?,
                cluster.name.as_str().into(),
                enum_text(&cluster.cni, "cni")?.into(),
                cluster.oidc.as_deref().into(),
                cluster.vault.as_deref().into(),
            ]),
        )
        .await
        .with_context(|| format!("Failed to store Talos cluster {}", cluster.name))?;
        for (role, members) in [
            ("controller", &cluster.controllers),
            ("worker", &cluster.workers),
        ] {
            for (member_index, member) in members.iter().enumerate() {
                execute(
                    conn,
                    definition::CREATE_TALOS_MEMBER.bind([
                        lab_key.into(),
                        position(index)?,
                        role.into(),
                        position(member_index)?,
                        member.as_str().into(),
                    ]),
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to store Talos cluster {} {role} {member}",
                        cluster.name
                    )
                })?;
            }
        }
    }
    Ok(())
}

/// The node's one-per-node parameters in `update_node.sql`'s order, under `key`.
fn node_values(key: &str, node: &Node) -> Result<[Value; 27]> {
    let topology = node.cpu.topology.as_ref();
    let vm = node.vm.as_ref();
    let container = node.container.as_ref();
    Ok([
        key.into(),
        node.image.as_deref().into(),
        node.version.as_deref().into(),
        node.memory.into(),
        node.cpu.count.into(),
        optional_enum(node.cpu.mode.as_ref(), "cpu.mode")?,
        topology.map(|topology| topology.sockets).into(),
        topology.map(|topology| topology.cores).into(),
        topology.map(|topology| topology.threads).into(),
        node.numa.node.into(),
        optional_enum(node.numa.memory_mode.as_ref(), "numa.memory_mode")?,
        node.boot_disk.size.into(),
        optional_enum(
            node.boot_disk.clone_method.as_ref(),
            "boot_disk.clone_method",
        )?,
        optional_enum(node.boot_disk.cache.as_ref(), "boot_disk.cache")?,
        optional_enum(node.boot_disk.io.as_ref(), "boot_disk.io")?,
        node.interfaces.data_count.into(),
        node.interfaces.queues.into(),
        optional_enum(node.ztp.method.as_ref(), "ztp.method")?,
        node.ztp.config.as_deref().into(),
        node.ready_check.port.into(),
        node.ready_check.skip.into(),
        optional_enum(vm.and_then(|vm| vm.watchdog.as_ref()), "vm.watchdog")?,
        container
            .and_then(|container| container.user.as_deref())
            .into(),
        container.and_then(|container| container.privileged).into(),
        container.and_then(|container| container.shm_size).into(),
        optional_enum(
            container.and_then(|container| container.platform.as_ref()),
            "container.platform",
        )?,
        node.unikernel
            .as_ref()
            .and_then(|unikernel| unikernel.cmdline.as_deref())
            .into(),
    ])
}

/// The node's lists of text values, by the name `manifest_node_text` gives each.
fn text_lists(node: &Node) -> Vec<(&'static str, Vec<&str>)> {
    vec![
        (
            "authorized_key",
            node.ssh
                .authorized_keys
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
        ),
        (
            "authorized_key_file",
            node.ssh
                .authorized_key_files
                .iter()
                .flatten()
                .map(|file| file.source.as_str())
                .collect(),
        ),
        (
            "container_command",
            node.container
                .iter()
                .flat_map(|container| container.commands.iter().flatten())
                .map(String::as_str)
                .collect(),
        ),
        (
            "environment_variable",
            node.environment_variables
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
        ),
        (
            "startup_script",
            node.startup_scripts
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
        ),
        (
            "user_script",
            node.user_scripts
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
        ),
        (
            "binary_file",
            node.binary_files
                .iter()
                .flatten()
                .map(|file| file.source.as_str())
                .collect(),
        ),
    ]
}

/// Replace the definition of the node `key` with `node`.
async fn write_node(conn: &Connection, key: &str, node: &Node) -> Result<()> {
    let changed = execute(conn, definition::UPDATE_NODE.bind(node_values(key, node)?))
        .await
        .with_context(|| format!("Failed to store node '{}'", node.name))?;
    if changed != 1 {
        bail!("Node '{}' not found", node.name);
    }
    for delete in [
        definition::DELETE_NODE_CPUS,
        definition::DELETE_NODE_TEXTS,
        definition::DELETE_NODE_VOLUMES,
        definition::DELETE_NODE_TEXT_FILES,
        definition::DELETE_NODE_SYSTEMD_UNITS,
        definition::DELETE_NODE_PROVISIONERS,
    ] {
        execute(conn, delete.bind([key.into()])).await?;
    }
    write_node_lists(conn, key, node)
        .await
        .with_context(|| format!("Failed to store the lists of node '{}'", node.name))?;
    write_node_files(conn, key, node)
        .await
        .with_context(|| format!("Failed to store the files of node '{}'", node.name))?;
    write_provisioners(conn, key, node)
        .await
        .with_context(|| format!("Failed to store the provisioners of node '{}'", node.name))
}

/// The node's lists of CPUs, of text values, and its container volumes.
async fn write_node_lists(conn: &Connection, key: &str, node: &Node) -> Result<()> {
    for (list, cpus) in [
        ("pinning", &node.cpu.pinning),
        ("emulator_pinning", &node.cpu.emulator_pinning),
        ("io_thread", &node.boot_disk.io_thread_cpus),
    ] {
        for (index, cpu) in cpus.iter().flatten().enumerate() {
            execute(
                conn,
                definition::CREATE_NODE_CPU.bind([
                    key.into(),
                    list.into(),
                    position(index)?,
                    (*cpu).into(),
                ]),
            )
            .await?;
        }
    }
    for (list, values) in text_lists(node) {
        for (index, value) in values.into_iter().enumerate() {
            execute(
                conn,
                definition::CREATE_NODE_TEXT.bind([
                    key.into(),
                    list.into(),
                    position(index)?,
                    value.into(),
                ]),
            )
            .await?;
        }
    }
    let volumes = node
        .container
        .iter()
        .flat_map(|container| container.volumes.iter().flatten());
    for (index, volume) in volumes.enumerate() {
        execute(
            conn,
            definition::CREATE_NODE_VOLUME.bind([
                key.into(),
                position(index)?,
                volume.src.as_str().into(),
                volume.dst.as_str().into(),
            ]),
        )
        .await?;
    }
    Ok(())
}

/// The node's `text_files` and `systemd_units`: the paths and settings the manifest gives.
async fn write_node_files(conn: &Connection, key: &str, node: &Node) -> Result<()> {
    for (index, file) in node.text_files.iter().flatten().enumerate() {
        execute(
            conn,
            definition::CREATE_NODE_TEXT_FILE.bind([
                key.into(),
                position(index)?,
                file.src.as_str().into(),
                file.dst.as_str().into(),
                file.user.as_str().into(),
                file.group.as_str().into(),
                file.permissions.into(),
            ]),
        )
        .await?;
    }
    for (index, unit) in node.systemd_units.iter().flatten().enumerate() {
        execute(
            conn,
            definition::CREATE_NODE_SYSTEMD_UNIT.bind([
                key.into(),
                position(index)?,
                unit.name.as_str().into(),
                unit.source.as_str().into(),
                unit.enabled.into(),
            ]),
        )
        .await?;
    }
    Ok(())
}

/// The node's provisioners, their files, and an ssh provisioner's env files and scripts.
async fn write_provisioners(conn: &Connection, key: &str, node: &Node) -> Result<()> {
    for (index, provisioner) in node.provisioners.iter().flatten().enumerate() {
        let kind = match provisioner {
            Provisioner::Ssh(_) => "ssh",
            Provisioner::Container(_) => "container",
        };
        execute(
            conn,
            definition::CREATE_NODE_PROVISIONER.bind([key.into(), position(index)?, kind.into()]),
        )
        .await?;
        match provisioner {
            Provisioner::Ssh(ssh) => {
                for (file_index, file) in ssh.files.iter().enumerate() {
                    execute(
                        conn,
                        definition::CREATE_NODE_PROVISIONER_FILE.bind([
                            key.into(),
                            position(index)?,
                            position(file_index)?,
                            file.src.as_str().into(),
                            file.dst.as_str().into(),
                            Value::Null,
                            file.mode.into(),
                        ]),
                    )
                    .await?;
                }
                for (list, paths) in [("env_file", &ssh.env_files), ("script", &ssh.scripts)] {
                    for (path_index, path) in paths.iter().enumerate() {
                        execute(
                            conn,
                            definition::CREATE_NODE_PROVISIONER_PATH.bind([
                                key.into(),
                                position(index)?,
                                list.into(),
                                position(path_index)?,
                                path.as_str().into(),
                            ]),
                        )
                        .await?;
                    }
                }
            }
            Provisioner::Container(container) => {
                for (file_index, file) in container.files.iter().enumerate() {
                    execute(
                        conn,
                        definition::CREATE_NODE_PROVISIONER_FILE.bind([
                            key.into(),
                            position(index)?,
                            position(file_index)?,
                            file.src.as_str().into(),
                            Value::Null,
                            file.name.as_str().into(),
                            file.mode.into(),
                        ]),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}
