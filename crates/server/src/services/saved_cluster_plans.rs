//! The one-time move of the Talos cluster plans older releases saved in each cluster's folder
//! on the server, `clusters/<cluster>/cluster.toml` in the lab's folder.
//!
//! A plan is the manifest's cluster with the addresses the server chose for it at `up`. The
//! lab's definition holds the manifest's side and the node records the members' addresses, so
//! only what the server chose is copied into the database: the endpoint, the shared address and
//! the load balancer addresses. At startup sherpad stores each lab's saved plans in one
//! transaction, then deletes the files. A file not there is skipped, so a second start does
//! nothing, and a start after a crash between the two stores the same values again. The
//! cluster's secrets and the user's talosconfig and kubeconfig stay in its folder.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::instrument;

use db::Database;
use shared::data::{ClusterPlan, DbLab};

use crate::services::talos::secrets::CLUSTERS_DIR;

/// The file older releases saved each cluster's plan in, in the cluster's folder.
const SAVED_PLAN_FILE: &str = "cluster.toml";

/// A lab's saved plan files, read.
struct SavedPlans {
    /// The plans the lab's definition has a cluster for, each with its file.
    plans: Vec<(PathBuf, ClusterPlan)>,
    /// The files that hold nothing to store.
    unusable: Vec<PathBuf>,
}

/// What the startup move did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SavedClusterPlansSummary {
    /// Labs whose saved plans were stored.
    pub stored: usize,
    /// Labs whose plans were kept because storing them failed. Each is logged; the next start
    /// tries again.
    pub kept: usize,
    /// Plan files deleted.
    pub deleted: usize,
}

/// Store the saved plans of every lab under `labs` with its definition, one transaction per
/// lab, then delete them. A plan that cannot be read, or names a cluster the lab's definition
/// does not have, is logged at warn with the lab and deleted: it holds nothing the database can
/// take. A lab whose plans cannot be stored keeps them for the next start. Folders of labs with
/// no record are left alone.
///
/// # Errors
///
/// Fails when the labs cannot be listed; nothing is deleted then.
#[instrument(skip(db), fields(labs = %labs.display()))]
pub async fn move_saved_cluster_plans(
    db: &Database,
    labs: &Path,
) -> Result<SavedClusterPlansSummary> {
    let mut summary = SavedClusterPlansSummary::default();
    for lab in db::list_labs(db).await? {
        let files = saved_plan_files(&labs.join(&lab.lab_id).join(CLUSTERS_DIR), &lab.lab_id);
        if files.is_empty() {
            continue;
        }
        let SavedPlans { plans, unusable } = match read_saved_plans(db, &lab, files).await {
            Ok(read) => read,
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    error = format!("{error:#}"),
                    "The lab's saved cluster plans cannot be stored; they are kept until the \
                     next start"
                );
                summary.kept += 1;
                continue;
            }
        };
        summary.deleted += delete_saved_plans(&unusable, &lab.lab_id);
        if plans.is_empty() {
            continue;
        }
        let (files, plans): (Vec<PathBuf>, Vec<ClusterPlan>) = plans.into_iter().unzip();
        let stored = match db::get_lab_id(&lab) {
            Ok(lab_record_id) => db::store_talos_cluster_plans(db, &lab_record_id, &plans).await,
            Err(error) => Err(error),
        };
        match stored {
            Ok(()) => {
                tracing::info!(lab_id = %lab.lab_id, clusters = plans.len(), "Stored the lab's saved cluster plans");
                summary.stored += 1;
                summary.deleted += delete_saved_plans(&files, &lab.lab_id);
            }
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    error = format!("{error:#}"),
                    "The lab's saved cluster plans cannot be stored; they are kept until the \
                     next start"
                );
                summary.kept += 1;
            }
        }
    }
    Ok(summary)
}

/// The saved plan files in `clusters_dir`, the clusters folder of lab `lab_id`: the
/// `cluster.toml` of each cluster folder that has one. A folder that cannot be listed is
/// logged and skipped.
fn saved_plan_files(clusters_dir: &Path, lab_id: &str) -> Vec<PathBuf> {
    let entries = match fs::read_dir(clusters_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(%lab_id, path = %clusters_dir.display(), %error, "Failed to list the lab's clusters");
            return Vec::new();
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let file = entry.path().join(SAVED_PLAN_FILE);
                if fs::symlink_metadata(&file).is_ok() {
                    files.push(file);
                }
            }
            Err(error) => {
                tracing::warn!(%lab_id, path = %clusters_dir.display(), %error, "Failed to list the lab's clusters");
            }
        }
    }
    files.sort();
    files
}

/// Read the saved plan `files` of `lab`. A file that holds nothing to store is logged at warn.
///
/// # Errors
///
/// Fails when the lab's definition cannot be read.
#[instrument(skip(db, lab, files), fields(lab_id = %lab.lab_id), level = "debug")]
async fn read_saved_plans(db: &Database, lab: &DbLab, files: Vec<PathBuf>) -> Result<SavedPlans> {
    let definition = db::read_lab_definition(db, &db::get_lab_id(lab)?)
        .await
        .context("Failed to read the lab's definition")?;
    let clusters: HashSet<String> = definition
        .and_then(|manifest| manifest.talos_clusters)
        .unwrap_or_default()
        .into_iter()
        .map(|cluster| cluster.name)
        .collect();
    let mut plans = Vec::new();
    let mut unusable = Vec::new();
    for file in files {
        let read = fs::read_to_string(&file)
            .with_context(|| format!("Unable to read {}", file.display()))
            .and_then(|contents| {
                toml::from_str::<ClusterPlan>(&contents)
                    .with_context(|| format!("{} is not a valid cluster plan", file.display()))
            });
        match read {
            Ok(plan) if clusters.contains(&plan.name) => plans.push((file, plan)),
            Ok(plan) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    cluster = %plan.name,
                    path = %file.display(),
                    "The saved cluster plan is for a cluster the lab's definition does not have; \
                     nothing in it can be stored, so it is deleted"
                );
                unusable.push(file);
            }
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    path = %file.display(),
                    error = format!("{error:#}"),
                    "The saved cluster plan cannot be read; nothing in it can be stored, so it is \
                     deleted"
                );
                unusable.push(file);
            }
        }
    }
    Ok(SavedPlans { plans, unusable })
}

/// Delete the saved plan `files` of lab `lab_id`. A file not there is skipped; one that cannot
/// be deleted is logged and left. Returns how many files were deleted.
fn delete_saved_plans(files: &[PathBuf], lab_id: &str) -> usize {
    let mut deleted = 0;
    for file in files {
        match fs::remove_file(file) {
            Ok(()) => {
                tracing::info!(%lab_id, path = %file.display(), "Deleted a saved cluster plan the database now holds");
                deleted += 1;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%lab_id, path = %file.display(), %error, "Failed to delete a saved cluster plan");
            }
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use shared::data::{ClusterCni, ClusterPlan, DbLab};

    use crate::services::talos::lab::tests::{cluster_lab, cluster_plans, save_cluster_files};
    use crate::services::talos::secrets::{
        CLUSTERS_DIR, KUBECONFIG_FILE, TALOSCONFIG_FILE, cluster_dir,
    };

    use super::*;

    /// The plan file 0.6.4 saved in each cluster's folder.
    const PLAN_FILE: &str = "cluster.toml";

    /// A temporary labs folder holding, for each of `cluster_plans`, the folder 0.6.4 left for
    /// it: its secrets, credentials and `cluster.toml`, the plan in the format 0.6.4 wrote.
    fn labs_dir(lab: &DbLab) -> tempfile::TempDir {
        let labs = tempfile::tempdir().unwrap();
        save_cluster_files(labs.path(), &lab.lab_id, &cluster_plans());
        for plan in cluster_plans() {
            fs::write(
                plan_file(labs.path(), lab, &plan.name),
                toml::to_string(&plan).unwrap(),
            )
            .unwrap();
        }
        labs
    }

    fn plan_file(labs: &Path, lab: &DbLab, cluster: &str) -> PathBuf {
        cluster_dir(labs, &lab.lab_id, cluster).join(PLAN_FILE)
    }

    async fn stored_plans(db: &Database, lab: &DbLab) -> Vec<ClusterPlan> {
        db::list_talos_cluster_plans(db, lab.id.as_ref().unwrap())
            .await
            .unwrap()
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn the_saved_plans_are_stored_and_deleted_and_a_second_start_does_nothing() {
        let (db, lab) = cluster_lab().await;
        let labs = labs_dir(&lab);

        let summary = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedClusterPlansSummary {
                stored: 1,
                kept: 0,
                deleted: 2
            }
        );
        assert_eq!(stored_plans(&db, &lab).await, cluster_plans());
        for plan in cluster_plans() {
            assert_eq!(
                files_in(&cluster_dir(labs.path(), &lab.lab_id, &plan.name)),
                [KUBECONFIG_FILE, "secrets.toml", TALOSCONFIG_FILE]
            );
        }

        let again = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(again, SavedClusterPlansSummary::default());
        assert_eq!(stored_plans(&db, &lab).await, cluster_plans());
    }

    #[tokio::test]
    async fn a_plan_left_by_a_crash_after_storing_is_stored_again_and_deleted() {
        let (db, lab) = cluster_lab().await;
        let labs = labs_dir(&lab);
        db::store_talos_cluster_plans(&db, lab.id.as_ref().unwrap(), &cluster_plans())
            .await
            .unwrap();
        fs::remove_file(plan_file(labs.path(), &lab, "ha")).unwrap();

        let summary = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedClusterPlansSummary {
                stored: 1,
                kept: 0,
                deleted: 1
            }
        );
        assert_eq!(stored_plans(&db, &lab).await, cluster_plans());
        assert!(!plan_file(labs.path(), &lab, "solo").exists());
    }

    #[tokio::test]
    async fn a_plan_that_does_not_parse_is_deleted_and_the_others_are_stored() {
        let (db, lab) = cluster_lab().await;
        let labs = labs_dir(&lab);
        fs::write(plan_file(labs.path(), &lab, "ha"), "name = [").unwrap();

        let summary = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(
            summary,
            SavedClusterPlansSummary {
                stored: 1,
                kept: 0,
                deleted: 2
            }
        );
        assert_eq!(stored_plans(&db, &lab).await, cluster_plans()[1..]);
        assert!(!plan_file(labs.path(), &lab, "ha").exists());
    }

    #[tokio::test]
    async fn a_plan_for_a_cluster_the_definition_lacks_is_deleted() {
        let (db, lab) = cluster_lab().await;
        let labs = labs_dir(&lab);
        let mut gone = cluster_plans()[1].clone();
        gone.name = "gone".to_owned();
        fs::create_dir_all(cluster_dir(labs.path(), &lab.lab_id, "gone")).unwrap();
        fs::write(
            plan_file(labs.path(), &lab, "gone"),
            toml::to_string(&gone).unwrap(),
        )
        .unwrap();

        let summary = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(summary.deleted, 3);
        assert_eq!(stored_plans(&db, &lab).await, cluster_plans());
        assert!(!plan_file(labs.path(), &lab, "gone").exists());
    }

    #[tokio::test]
    async fn plans_saved_before_the_cni_choice_are_stored_as_flannel_clusters() {
        let (db, lab) = cluster_lab().await;
        let labs = labs_dir(&lab);
        fs::write(
            plan_file(labs.path(), &lab, "solo"),
            "name = \"solo\"\nendpoint = \"172.31.1.25\"\nworkers = []\n\n\
             [[control_planes]]\nname = \"solo\"\nipv4 = \"172.31.1.25\"\n",
        )
        .unwrap();

        move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        let solo = &stored_plans(&db, &lab).await[1];
        assert_eq!(solo, &cluster_plans()[1]);
        assert_eq!(solo.cni, ClusterCni::Flannel);
    }

    #[tokio::test]
    async fn labs_without_saved_plans_and_folders_without_a_lab_are_left_alone() {
        let (db, lab) = cluster_lab().await;
        let labs = tempfile::tempdir().unwrap();
        save_cluster_files(labs.path(), &lab.lab_id, &cluster_plans());
        let stray = labs.path().join("lab-0002").join(CLUSTERS_DIR).join("k8s");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join(PLAN_FILE), "name = \"k8s\"").unwrap();

        let summary = move_saved_cluster_plans(&db, labs.path()).await.unwrap();

        assert_eq!(summary, SavedClusterPlansSummary::default());
        assert_eq!(stored_plans(&db, &lab).await, Vec::<ClusterPlan>::new());
        assert!(stray.join(PLAN_FILE).exists());
    }
}
