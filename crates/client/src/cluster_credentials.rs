use std::path::PathBuf;

use anyhow::Result;

use shared::data::ClusterInfo;
use shared::util::{Emoji, LabFolder};

use crate::lab_files::ensure_lab_subdir;
use crate::private_key::write_secret_file;

/// Files written for one cluster.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClusterFiles {
    pub name: String,
    pub kubeconfig: PathBuf,
    pub talosconfig: PathBuf,
    /// Addresses of a Cilium cluster's Gateway and Hubble UI.
    pub gateway: Option<String>,
    pub hubble_ui: Option<String>,
}

/// Print where each cluster's credentials were written.
pub(crate) fn print_cluster_files(files: &[ClusterFiles]) {
    for cluster in files {
        println!(
            "{} Cluster {} credentials: {} and {}",
            Emoji::Success,
            cluster.name,
            cluster.kubeconfig.display(),
            cluster.talosconfig.display()
        );
        println!(
            "   export KUBECONFIG={} TALOSCONFIG={}",
            cluster.kubeconfig.display(),
            cluster.talosconfig.display()
        );
        if let Some(gateway) = &cluster.gateway {
            println!("   Gateway: http://{gateway}");
        }
        if let Some(hubble_ui) = &cluster.hubble_ui {
            println!("   Hubble UI: http://{hubble_ui}");
        }
    }
}

/// Write each cluster's kubeconfig and talosconfig into `folder`, readable only by the current
/// user, as `.sherpa/k8s/kubeconfig-<cluster>` and `.sherpa/talos/talosconfig-<cluster>`.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display(), clusters = clusters.len()))]
pub(crate) fn write_cluster_credentials(
    folder: &LabFolder,
    clusters: &[ClusterInfo],
) -> Result<Vec<ClusterFiles>> {
    if !clusters.is_empty() {
        ensure_lab_subdir(folder, &folder.k8s_dir())?;
        ensure_lab_subdir(folder, &folder.talos_dir())?;
    }
    let mut written = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let kubeconfig = folder.kubeconfig(&cluster.name);
        let talosconfig = folder.talosconfig(&cluster.name);
        write_secret_file(&kubeconfig, &cluster.kubeconfig, "kubeconfig")?;
        write_secret_file(&talosconfig, &cluster.talosconfig, "talosconfig")?;
        written.push(ClusterFiles {
            name: cluster.name.clone(),
            kubeconfig,
            talosconfig,
            gateway: cluster.gateway.clone(),
            hubble_ui: cluster.hubble_ui.clone(),
        });
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cluster_gets_its_own_credential_files() {
        let dir = tempfile::tempdir().unwrap();
        let clusters = ["k8s", "edge"].map(|name| ClusterInfo {
            name: name.to_owned(),
            endpoint: format!("https://{name}:6443"),
            talosconfig: format!("talos-{name}"),
            kubeconfig: format!("kube-{name}"),
            gateway: None,
            hubble_ui: None,
        });
        let written = write_cluster_credentials(&LabFolder::new(dir.path()), &clusters).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".sherpa/k8s/kubeconfig-edge")).unwrap(),
            "kube-edge"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".sherpa/talos/talosconfig-k8s")).unwrap(),
            "talos-k8s"
        );
        assert_eq!(
            written[0].kubeconfig,
            dir.path().join(".sherpa/k8s/kubeconfig-k8s")
        );
    }
}
