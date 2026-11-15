//! The one-time removal of the files older releases left in each lab's folder on the server
//! that nothing reads:
//!
//! - `ztp/dnsmasq/dnsmasq.leases`: written empty, never mounted in the boot container.
//! - A unikernel's `certs/<node>.crt` and `certs/<node>.key`: issued, never carried to it.
//! - A VM's boot files, `<node>/`: read once, to upload its media to the storage pool.
//!
//! At startup sherpad removes them from the folder of every lab with a record. A file not there
//! is skipped, so a second start does nothing; nothing else in the folder is touched.

use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use crate::services::node_ops;

use db::Database;
use shared::data::{DbLab, NodeKind};
use shared::konst::{DNSMASQ_DIR, DNSMASQ_LEASES_FILE, LAB_CERTS_DIR, ZTP_DIR};

/// What the startup removal did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UnreadLabFilesSummary {
    /// Files and VM boot file folders removed.
    pub removed: usize,
    /// Labs whose nodes could not be listed, so only their leases file was looked at. Each is
    /// logged; the next start tries again.
    pub skipped: usize,
}

/// Remove the leases file, the unikernels' certificates and the VMs' boot files from the
/// folder of every lab under `labs` that has a record. One that cannot be removed is logged and
/// left for the next start. Folders of labs with no record are left alone.
///
/// # Errors
///
/// Fails when the labs cannot be listed; nothing is removed then.
#[instrument(skip(db), fields(labs = %labs.display()))]
pub async fn remove_unread_lab_files(db: &Database, labs: &Path) -> Result<UnreadLabFilesSummary> {
    let mut summary = UnreadLabFilesSummary::default();
    for lab in db::list_labs(db).await? {
        let lab_dir = labs.join(&lab.lab_id);
        if !lab_dir.is_dir() {
            continue;
        }
        let leases = lab_dir
            .join(ZTP_DIR)
            .join(DNSMASQ_DIR)
            .join(DNSMASQ_LEASES_FILE);
        summary.removed += usize::from(was_removed(&lab.lab_id, &leases, fs::remove_file(&leases)));
        match node_kinds(db, &lab).await {
            Ok(nodes) => summary.removed += remove_node_files(&lab.lab_id, &lab_dir, &nodes),
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    error = format!("{error:#}"),
                    "The lab's nodes cannot be listed; their unread files are kept until the \
                     next start"
                );
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

/// The name and kind of each of `lab`'s nodes.
///
/// # Errors
///
/// Fails when the nodes or their images cannot be read.
#[instrument(skip(db, lab), fields(lab_id = %lab.lab_id), level = "debug")]
async fn node_kinds(db: &Database, lab: &DbLab) -> Result<Vec<(String, NodeKind)>> {
    let nodes = db::list_nodes_by_lab(db, db::get_lab_id(lab)?)
        .await
        .context("Failed to list the lab's nodes")?;
    let images =
        db::list_node_images_by_ids(db, nodes.iter().map(|node| node.image.clone()).collect())
            .await
            .context("Failed to read the images of the lab's nodes")?;
    let kinds: HashMap<_, _> = images
        .into_iter()
        .filter_map(|image| image.id.map(|id| (id, image.kind)))
        .collect();
    Ok(nodes
        .into_iter()
        .filter_map(|node| kinds.get(&node.image).map(|kind| (node.name, kind.clone())))
        .collect())
}

/// Remove the unread files of `nodes` in `lab_dir`, the folder of lab `lab_id`: a unikernel's
/// certificate and key, a VM's boot files. Returns how many were removed.
#[instrument(skip(lab_dir, nodes), fields(lab_dir = %lab_dir.display()), level = "debug")]
fn remove_node_files(lab_id: &str, lab_dir: &Path, nodes: &[(String, NodeKind)]) -> usize {
    let certs_dir = lab_dir.join(LAB_CERTS_DIR);
    let mut removed = 0;
    for (name, kind) in nodes {
        match kind {
            NodeKind::Unikernel => {
                for extension in ["crt", "key"] {
                    let path = certs_dir.join(format!("{name}.{extension}"));
                    removed += usize::from(was_removed(lab_id, &path, fs::remove_file(&path)));
                }
            }
            NodeKind::VirtualMachine => {
                if let Some(dir) = node_ops::node_dir(lab_dir, name) {
                    removed += usize::from(was_removed(lab_id, &dir, fs::remove_dir_all(&dir)));
                }
            }
            NodeKind::Container => {}
        }
    }
    removed
}

/// Whether `path` was removed, from the `result` of removing it, which is logged. A path not
/// there was skipped; one that could not be removed is left.
fn was_removed(lab_id: &str, path: &Path, result: io::Result<()>) -> bool {
    match result {
        Ok(()) => {
            tracing::info!(%lab_id, path = %path.display(), "Removed a lab file nothing reads");
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(%lab_id, path = %path.display(), %error, "Failed to remove a lab file nothing reads");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use shared::data::NodeModel;
    use shared::konst::{DNSMASQ_CONFIG_FILE, LAB_CA_CERT_FILE, LAB_CA_KEY_FILE, TFTP_DIR};

    use crate::services::lab_definition::tests::lab_with;

    use super::*;

    /// A folder for `lab-0001` under a temporary labs directory, as 0.6.4 left it for a lab with
    /// a VM `vm1`, a unikernel `uk1` and a container `web`: the leases file, every node's
    /// certificate, the VM's boot files, the container's config mount and a TFTP config.
    fn labs_dir() -> (tempfile::TempDir, PathBuf) {
        let labs = tempfile::tempdir().unwrap();
        let lab_dir = labs.path().join("lab-0001");
        let dnsmasq = lab_dir.join(ZTP_DIR).join(DNSMASQ_DIR);
        let certs = lab_dir.join(LAB_CERTS_DIR);
        for dir in [
            &dnsmasq,
            &certs,
            &lab_dir.join(ZTP_DIR).join(TFTP_DIR),
            &lab_dir.join("vm1"),
            &lab_dir.join("web"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(dnsmasq.join(DNSMASQ_CONFIG_FILE), "port=53\n").unwrap();
        fs::write(dnsmasq.join(DNSMASQ_LEASES_FILE), "").unwrap();
        for file in [LAB_CA_CERT_FILE, LAB_CA_KEY_FILE] {
            fs::write(certs.join(file), "lab CA").unwrap();
        }
        for node in ["vm1", "uk1", "web"] {
            fs::write(certs.join(format!("{node}.crt")), "cert").unwrap();
            fs::write(certs.join(format!("{node}.key")), "key").unwrap();
        }
        fs::write(lab_dir.join("vm1").join("user-data"), "#cloud-config\n").unwrap();
        fs::write(lab_dir.join("vm1").join("ztp.iso"), "iso").unwrap();
        fs::write(lab_dir.join("web").join("startup.conf"), "hostname web\n").unwrap();
        fs::write(
            lab_dir.join(ZTP_DIR).join(TFTP_DIR).join("vm1.conf"),
            "hostname vm1\n",
        )
        .unwrap();
        (labs, lab_dir)
    }

    /// Every file under `dir`, relative to it, sorted.
    fn files_under(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in fs::read_dir(next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(
                        path.strip_prefix(dir)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        found.sort();
        found
    }

    async fn lab() -> Database {
        lab_with(&[
            ("vm1", NodeModel::AlpineLinux),
            ("uk1", NodeModel::NanosUnikernel),
            ("web", NodeModel::GenericContainer),
        ])
        .await
        .0
    }

    #[tokio::test]
    async fn the_unread_files_of_an_existing_lab_are_removed_at_startup() {
        let db = lab().await;
        let (labs, lab_dir) = labs_dir();

        let summary = remove_unread_lab_files(&db, labs.path()).await.unwrap();

        // The leases file, the unikernel's certificate and key, and the VM's boot folder.
        assert_eq!(
            summary,
            UnreadLabFilesSummary {
                removed: 4,
                skipped: 0
            }
        );
        // What dnsmasq, QEMU and Docker read stays: the dnsmasq config, the TFTP config, the
        // lab CA, the container's and the VM's certificates, and the container's mount.
        assert_eq!(
            files_under(&lab_dir),
            [
                "certs/ca.crt",
                "certs/ca.key",
                "certs/vm1.crt",
                "certs/vm1.key",
                "certs/web.crt",
                "certs/web.key",
                "web/startup.conf",
                "ztp/dnsmasq/dnsmasq.conf",
                "ztp/tftp/vm1.conf",
            ]
        );
    }

    #[tokio::test]
    async fn a_second_start_removes_nothing() {
        let db = lab().await;
        let (labs, lab_dir) = labs_dir();
        remove_unread_lab_files(&db, labs.path()).await.unwrap();
        let after_first = files_under(&lab_dir);

        let summary = remove_unread_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(summary, UnreadLabFilesSummary::default());
        assert_eq!(files_under(&lab_dir), after_first);
    }

    #[tokio::test]
    async fn a_vm_named_like_a_lab_folder_keeps_that_folder() {
        let (db, _) = lab_with(&[("certs", NodeModel::AlpineLinux)]).await;
        let (labs, lab_dir) = labs_dir();

        remove_unread_lab_files(&db, labs.path()).await.unwrap();

        assert!(lab_dir.join(LAB_CERTS_DIR).join(LAB_CA_KEY_FILE).exists());
    }

    #[tokio::test]
    async fn the_folder_of_a_lab_with_no_record_is_left_alone() {
        let db = lab().await;
        let (labs, _) = labs_dir();
        let other = labs.path().join("ffff0000");
        let leases = other.join(ZTP_DIR).join(DNSMASQ_DIR);
        fs::create_dir_all(&leases).unwrap();
        fs::write(leases.join(DNSMASQ_LEASES_FILE), "").unwrap();

        remove_unread_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(files_under(&other), ["ztp/dnsmasq/dnsmasq.leases"]);
    }

    #[tokio::test]
    async fn a_lab_with_no_folder_is_skipped() {
        let db = lab().await;
        let labs = tempfile::tempdir().unwrap();

        let summary = remove_unread_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(summary, UnreadLabFilesSummary::default());
    }
}
