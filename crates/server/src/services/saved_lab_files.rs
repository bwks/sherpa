//! The one-time move of the lab files older releases kept in each lab's folder on the server:
//! `lab-info.toml`, the SSH config and `known_hosts`.
//!
//! Everything they held is in the database now: the lab record holds the lab's details, and
//! each node record its management address, its host key and the account the SSH config logs
//! in to it with. That account, `ssh_usernames` in `lab-info.toml`, was in the file only. At
//! startup sherpad copies it into the node records and deletes the three files. A file not
//! there is skipped, so a second start does nothing; nothing else in the folder is touched.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use db::Database;
use shared::data::{DbLab, LabInfo, NodeSshUsername};
use shared::konst::{LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_SSH_CONFIG_FILE};

/// The files older releases kept in a lab's folder that the database now holds.
const SAVED_LAB_FILES: [&str; 3] = [
    LAB_FILE_NAME,
    SHERPA_SSH_CONFIG_FILE,
    SHERPA_KNOWN_HOSTS_FILE,
];

/// What the startup move did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SavedLabFilesSummary {
    /// Labs whose SSH accounts were copied from their `lab-info.toml`.
    pub stored: usize,
    /// Labs whose files were kept because storing their SSH accounts failed. Each is logged;
    /// the next start tries again.
    pub kept: usize,
    /// Files deleted.
    pub deleted: usize,
}

/// What became of one lab's `lab-info.toml`.
#[derive(Debug, PartialEq, Eq)]
enum LabOutcome {
    /// The lab has none.
    NoLabInfo,
    /// Its SSH accounts were stored, this many changed.
    Stored(usize),
    /// It could not be read, so nothing in it could be stored.
    Unreadable,
}

/// Copy the SSH accounts of every lab's `lab-info.toml` under `labs` into its node records,
/// then delete the lab's `lab-info.toml`, SSH config and `known_hosts`. A file that cannot be
/// read is logged at warn and deleted too: it holds nothing the database can take. A lab whose
/// accounts cannot be stored keeps its files for the next start. Folders of labs with no
/// record are left alone.
///
/// # Errors
///
/// Fails when the labs cannot be listed; nothing is deleted then.
#[instrument(skip(db), fields(labs = %labs.display()))]
pub async fn move_saved_lab_files(db: &Database, labs: &Path) -> Result<SavedLabFilesSummary> {
    let mut summary = SavedLabFilesSummary::default();
    for lab in db::list_labs(db).await? {
        let lab_dir = labs.join(&lab.lab_id);
        match store_saved_ssh_usernames(db, &lab, &lab_dir).await {
            Ok(LabOutcome::NoLabInfo | LabOutcome::Unreadable) => {}
            Ok(LabOutcome::Stored(changed)) => {
                tracing::info!(lab_id = %lab.lab_id, changed, "Stored the lab's SSH accounts from its lab-info.toml");
                summary.stored += 1;
            }
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    lab_dir = %lab_dir.display(),
                    error = format!("{error:#}"),
                    "The lab's lab-info.toml cannot be stored; its lab files are kept until the \
                     next start"
                );
                summary.kept += 1;
                continue;
            }
        }
        summary.deleted += delete_saved_lab_files(&lab_dir, &lab.lab_id);
    }
    Ok(summary)
}

/// Store the SSH accounts of the `lab-info.toml` in `lab_dir` in `lab`'s node records. An
/// account for a node the lab no longer has is skipped.
///
/// # Errors
///
/// Fails when the records cannot be read or written.
#[instrument(skip(db, lab), fields(lab_id = %lab.lab_id), level = "debug")]
async fn store_saved_ssh_usernames(
    db: &Database,
    lab: &DbLab,
    lab_dir: &Path,
) -> Result<LabOutcome> {
    let nodes = db::list_nodes_by_lab(db, db::get_lab_id(lab)?)
        .await
        .context("Failed to list the lab's nodes")?;
    let usernames = match saved_ssh_usernames(lab_dir) {
        Ok(Some(usernames)) => usernames,
        Ok(None) => return Ok(LabOutcome::NoLabInfo),
        Err(error) => {
            tracing::warn!(
                error = format!("{error:#}"),
                "The lab's lab-info.toml cannot be read; nothing in it can be stored, so it is \
                 deleted"
            );
            return Ok(LabOutcome::Unreadable);
        }
    };
    let mut changed = 0;
    for entry in usernames {
        let Some(node) = nodes.iter().find(|node| node.name == entry.node_name) else {
            tracing::debug!(node_name = %entry.node_name, "SSH account for a node no longer in the lab");
            continue;
        };
        if node.ssh_username.as_deref() == Some(entry.username.as_str()) {
            continue;
        }
        db::update_node_ssh_username(db, db::get_node_id(node)?, Some(&entry.username))
            .await
            .with_context(|| format!("Failed to store the SSH account of '{}'", node.name))?;
        changed += 1;
    }
    Ok(LabOutcome::Stored(changed))
}

/// The SSH accounts in the `lab-info.toml` in `lab_dir`, or `None` when there is none.
///
/// # Errors
///
/// Fails when the file is there but cannot be read or parsed.
fn saved_ssh_usernames(lab_dir: &Path) -> Result<Option<Vec<NodeSshUsername>>> {
    let path = lab_dir.join(LAB_FILE_NAME);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Unable to read {}", path.display()));
        }
    };
    let lab_info: LabInfo = contents
        .parse()
        .with_context(|| format!("{} is not a valid lab info file", path.display()))?;
    Ok(Some(lab_info.ssh_usernames))
}

/// Delete the `lab-info.toml`, SSH config and `known_hosts` in `lab_dir`, the folder of lab
/// `lab_id`, and nothing else. A file not there is skipped; one that cannot be deleted is
/// logged and left. Returns how many files were deleted.
#[instrument(skip(lab_dir), fields(lab_dir = %lab_dir.display()), level = "debug")]
fn delete_saved_lab_files(lab_dir: &Path, lab_id: &str) -> usize {
    let mut deleted = 0;
    for file in SAVED_LAB_FILES {
        let path = lab_dir.join(file);
        match fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(%lab_id, path = %path.display(), "Deleted a lab file the database now holds");
                deleted += 1;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%lab_id, path = %path.display(), %error, "Failed to delete a saved lab file");
            }
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use shared::data::NodeModel;
    use shared::konst::{LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE};

    use crate::services::lab_definition::tests::lab_with;

    use super::*;

    /// The `lab-info.toml` 0.6.4 wrote for `lab-0001`, with these SSH accounts: the lab info
    /// it wrote with `LabInfo`'s own format.
    fn lab_info_toml(accounts: &[(&str, &str)]) -> String {
        let mut lab_info: LabInfo = "id = \"lab-0001\"\nname = \"core\"\nuser = \"alice\"\n\
             ipv4_network = \"172.31.1.0/24\"\nipv4_gateway = \"172.31.1.1\"\n\
             ipv4_router = \"172.31.1.2\"\nloopback_network = \"127.127.1.0/24\"\n"
            .parse()
            .unwrap();
        for (node_name, username) in accounts {
            lab_info.set_ssh_username(node_name, Some(username));
        }
        lab_info.to_string()
    }

    /// A folder for `lab-0001` under a temporary labs directory, holding the three saved files
    /// with `lab_info` as its `lab-info.toml`, and the lab's key and CA.
    fn labs_dir(lab_info: &str) -> (tempfile::TempDir, PathBuf) {
        let labs = tempfile::tempdir().unwrap();
        let lab_dir = labs.path().join("lab-0001");
        fs::create_dir_all(lab_dir.join(LAB_CERTS_DIR)).unwrap();
        fs::write(lab_dir.join(LAB_FILE_NAME), lab_info).unwrap();
        fs::write(lab_dir.join(SHERPA_SSH_CONFIG_FILE), "Host vm1\n").unwrap();
        fs::write(
            lab_dir.join(SHERPA_KNOWN_HOSTS_FILE),
            "172.31.1.10 ssh-ed25519 A\n",
        )
        .unwrap();
        fs::write(lab_dir.join(SHERPA_SSH_PRIVATE_KEY_FILE), "lab key").unwrap();
        fs::write(lab_dir.join(LAB_CERTS_DIR).join("ca.crt"), "lab CA").unwrap();
        (labs, lab_dir)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    async fn stored_accounts(db: &Database, lab: &DbLab) -> Vec<(String, Option<String>)> {
        db::list_nodes_by_lab(db, lab.id.clone().unwrap())
            .await
            .unwrap()
            .into_iter()
            .map(|node| (node.name, node.ssh_username))
            .collect()
    }

    #[tokio::test]
    async fn the_ssh_accounts_are_stored_and_the_files_deleted() {
        let (db, lab) = lab_with(&[
            ("vm1", NodeModel::AlpineLinux),
            ("web", NodeModel::GenericContainer),
        ])
        .await;
        let (labs, lab_dir) = labs_dir(&lab_info_toml(&[("vm1", "alice"), ("gone", "bob")]));

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedLabFilesSummary {
                stored: 1,
                kept: 0,
                deleted: 3
            }
        );
        assert_eq!(
            stored_accounts(&db, &lab).await,
            [
                ("vm1".to_owned(), Some("alice".to_owned())),
                ("web".to_owned(), None)
            ]
        );
        // Only the three files go: the lab's key and certificates stay.
        assert_eq!(
            files_in(&lab_dir),
            [LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE]
        );
        assert_eq!(files_in(&lab_dir.join(LAB_CERTS_DIR)), ["ca.crt"]);
    }

    #[tokio::test]
    async fn a_second_start_does_nothing() {
        let (db, lab) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let (labs, lab_dir) = labs_dir(&lab_info_toml(&[("vm1", "alice")]));
        move_saved_lab_files(&db, labs.path()).await.unwrap();

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(summary, SavedLabFilesSummary::default());
        assert_eq!(
            stored_accounts(&db, &lab).await,
            [("vm1".to_owned(), Some("alice".to_owned()))]
        );
        assert_eq!(
            files_in(&lab_dir),
            [LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE]
        );
    }

    #[tokio::test]
    async fn an_unreadable_lab_info_is_deleted_when_every_node_uses_the_sherpa_account() {
        let (db, lab) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let (labs, lab_dir) = labs_dir("not [[ a lab info");

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedLabFilesSummary {
                stored: 0,
                kept: 0,
                deleted: 3
            }
        );
        assert_eq!(stored_accounts(&db, &lab).await, [("vm1".to_owned(), None)]);
        assert_eq!(
            files_in(&lab_dir),
            [LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE]
        );
    }

    #[tokio::test]
    async fn an_unreadable_lab_info_is_deleted_when_the_database_has_the_accounts() {
        let (db, lab) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let node = &db::list_nodes_by_lab(&db, lab.id.clone().unwrap())
            .await
            .unwrap()[0];
        db::update_node_ssh_username(&db, node.id.clone().unwrap(), Some("alice"))
            .await
            .unwrap();
        let (labs, lab_dir) = labs_dir("not [[ a lab info");

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedLabFilesSummary {
                stored: 0,
                kept: 0,
                deleted: 3
            }
        );
        assert_eq!(
            stored_accounts(&db, &lab).await,
            [("vm1".to_owned(), Some("alice".to_owned()))]
        );
        assert_eq!(
            files_in(&lab_dir),
            [LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE]
        );
    }

    #[tokio::test]
    async fn ssh_files_without_a_lab_info_are_deleted() {
        let (db, _) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let (labs, lab_dir) = labs_dir("");
        fs::remove_file(lab_dir.join(LAB_FILE_NAME)).unwrap();

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(summary.deleted, 2);
        assert_eq!(
            files_in(&lab_dir),
            [LAB_CERTS_DIR, SHERPA_SSH_PRIVATE_KEY_FILE]
        );
    }

    #[tokio::test]
    async fn the_folder_of_a_lab_with_no_record_is_left_alone() {
        let (db, _) = lab_with(&[]).await;
        let (labs, _) = labs_dir(&lab_info_toml(&[]));
        let other = labs.path().join("ffff0000");
        fs::create_dir(&other).unwrap();
        fs::write(other.join(LAB_FILE_NAME), lab_info_toml(&[])).unwrap();

        move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(files_in(&other), [LAB_FILE_NAME]);
    }

    #[tokio::test]
    async fn a_lab_with_no_folder_is_skipped() {
        let (db, _) = lab_with(&[("vm1", NodeModel::AlpineLinux)]).await;
        let labs = tempfile::tempdir().unwrap();

        let summary = move_saved_lab_files(&db, labs.path()).await.unwrap();

        assert_eq!(summary, SavedLabFilesSummary::default());
    }
}
