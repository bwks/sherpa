//! Stages 2 and 3: destroying the node, and recreating a VM's own networks.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use super::context::Redeploy;
use crate::services::container_vm;
use crate::services::error::ServiceError;
use crate::services::node_ops;

use shared::data;
use shared::data::StatusKind;

/// Destroy the node and its configuration directory (stage 2).
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn destroy_node(redeploy: &Redeploy<'_>) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let _ = redeploy.progress.send_status(
        format!("Destroying existing node: {node_name}"),
        StatusKind::Progress,
    );

    match redeploy.node_image.kind() {
        data::NodeKind::VirtualMachine => {
            node_ops::destroy_vm_node(
                redeploy.qemu_conn.clone(),
                node_name,
                lab_id,
                redeploy.db_node.index,
                redeploy.node_image.interfaces.reserved_count,
            )
            .await?;
        }
        data::NodeKind::Container if redeploy.target_node.model_kind.runs_in_vm() => {
            container_vm::remove_container_vm(
                &redeploy.state.qemu,
                &container_vm::domain_name(node_name, lab_id),
            )
            .await?;
            node_ops::destroy_container_node(&redeploy.state.docker, node_name, lab_id).await?;
        }
        data::NodeKind::Container => {
            node_ops::destroy_container_node(&redeploy.state.docker, node_name, lab_id).await?;
        }
        data::NodeKind::Unikernel => {
            return Err(ServiceError::Invalid(
                "Redeploy not supported for unikernel nodes".to_owned(),
            )
            .into());
        }
    }

    remove_node_folders(
        Path::new(&redeploy.lab_dir),
        node_name,
        redeploy.node_image.kind(),
    )?;

    let _ = redeploy.progress.send_status(
        format!("Existing node destroyed: {node_name}"),
        StatusKind::Done,
    );
    Ok(())
}

/// Remove the folders of node `node_name`, of `kind`, in the lab folder `lab_dir`: the files it
/// was built from, which the rebuild writes again. A VM's boot folder goes, and the node's own
/// folder, a container's config mounts or a VM's boot files from a release before this one. A
/// node named like one of the lab's own folders keeps that folder: it holds the lab's CA,
/// credentials or boot server files (`node_ops::node_dir`).
///
/// # Errors
///
/// Fails when a folder is there but cannot be removed.
fn remove_node_folders(lab_dir: &Path, node_name: &str, kind: data::NodeKind) -> Result<()> {
    if matches!(kind, data::NodeKind::VirtualMachine) {
        let lab_dir = lab_dir.to_str().context("Lab folder path is not UTF-8")?;
        node_ops::remove_vm_boot_dir(Path::new(&node_ops::vm_boot_dir(lab_dir, node_name)))?;
    }
    match node_ops::node_dir(lab_dir, node_name) {
        Some(dir) if dir.exists() => fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to remove ZTP dir: {}", dir.display())),
        Some(_) => Ok(()),
        None => {
            tracing::warn!(
                node_name,
                "The node is named like one of the lab's own folders; that folder is kept"
            );
            Ok(())
        }
    }
}

/// Recreate a VM's isolated network, and its reserved network if it has one (stage 3).
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) fn recreate_vm_networks(redeploy: &Redeploy<'_>) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let node_idx = redeploy.db_node.index;
    let _ = redeploy.progress.send_status(
        format!("Recreating networks for node: {node_name}"),
        StatusKind::Progress,
    );

    let isolated = node_ops::node_isolated_network_data(node_name, node_idx, lab_id);
    libvirt::IsolatedNetwork {
        network_name: isolated.network_name,
        bridge_name: isolated.bridge_name,
    }
    .create(&redeploy.qemu_conn)?;

    if redeploy.node_image.interfaces.reserved_count > 0 {
        let reserved = node_ops::node_reserved_network_data(node_name, node_idx, lab_id);
        libvirt::ReservedNetwork {
            network_name: reserved.network_name,
            bridge_name: reserved.bridge_name,
        }
        .create(&redeploy.qemu_conn)?;
    }

    let _ = redeploy.progress.send_status(
        format!("Networks recreated for node: {node_name}"),
        StatusKind::Done,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::konst::{LAB_CA_KEY_FILE, LAB_CERTS_DIR};
    use shared::data::NodeKind;
    use shared::konst::ZTP_DIR;

    use crate::services::keycloak::lab::KEYCLOAK_DIR;
    use crate::services::surreal_db::SURREAL_DB_DIR;
    use crate::services::talos::secrets::CLUSTERS_DIR;
    use crate::services::vault::lab::VAULT_DIR;

    /// A lab folder holding a file in each of the lab's own folders.
    fn lab_folder() -> tempfile::TempDir {
        let lab = tempfile::tempdir().unwrap();
        for dir in [
            LAB_CERTS_DIR,
            ZTP_DIR,
            CLUSTERS_DIR,
            VAULT_DIR,
            KEYCLOAK_DIR,
            SURREAL_DB_DIR,
        ] {
            fs::create_dir(lab.path().join(dir)).unwrap();
            fs::write(lab.path().join(dir).join(LAB_CA_KEY_FILE), "lab secret").unwrap();
        }
        lab
    }

    #[test]
    fn config_privacy_007_a_node_named_like_a_lab_folder_keeps_that_folder() {
        for kind in [NodeKind::VirtualMachine, NodeKind::Container] {
            for name in [
                LAB_CERTS_DIR,
                ZTP_DIR,
                CLUSTERS_DIR,
                VAULT_DIR,
                KEYCLOAK_DIR,
                SURREAL_DB_DIR,
            ] {
                let lab = lab_folder();

                remove_node_folders(lab.path(), name, kind).unwrap();

                assert!(
                    lab.path().join(name).join(LAB_CA_KEY_FILE).exists(),
                    "{kind:?} {name}"
                );
            }
        }
    }

    #[test]
    fn config_privacy_008_a_vm_named_like_a_lab_folder_still_loses_its_boot_files() {
        for name in [
            LAB_CERTS_DIR,
            ZTP_DIR,
            CLUSTERS_DIR,
            VAULT_DIR,
            KEYCLOAK_DIR,
            SURREAL_DB_DIR,
        ] {
            let lab = lab_folder();
            let boot_dir = node_ops::vm_boot_dir(lab.path().to_str().unwrap(), name);
            fs::create_dir_all(&boot_dir).unwrap();
            fs::write(Path::new(&boot_dir).join("user-data"), "#cloud-config\n").unwrap();

            remove_node_folders(lab.path(), name, NodeKind::VirtualMachine).unwrap();

            assert!(!Path::new(&boot_dir).exists(), "{name}");
            assert!(
                lab.path().join(name).join(LAB_CA_KEY_FILE).exists(),
                "{name}"
            );
        }
    }

    #[test]
    fn config_privacy_006_a_redeploy_removes_only_the_nodes_own_folders() {
        let lab = lab_folder();
        for dir in ["ct1", "ct10"] {
            fs::create_dir(lab.path().join(dir)).unwrap();
            fs::write(lab.path().join(dir).join("startup.conf"), "config").unwrap();
        }

        remove_node_folders(lab.path(), "ct1", NodeKind::Container).unwrap();

        assert!(!lab.path().join("ct1").exists());
        assert!(lab.path().join("ct10").join("startup.conf").exists());
    }
}
