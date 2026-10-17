use std::path::{Path, PathBuf};

use anyhow::Result;

use shared::data::ClusterInfo;
use shared::util::Emoji;

use crate::private_key::write_secret_file;

/// Files written for one cluster.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClusterFiles {
    pub name: String,
    pub kubeconfig: PathBuf,
    pub talosconfig: PathBuf,
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
    }
}

/// Write each cluster's kubeconfig and talosconfig into `dir`, readable only by the current
/// user, as `kubeconfig-<cluster>` and `talosconfig-<cluster>`.
#[tracing::instrument(level = "debug", skip(clusters), fields(dir = %dir.display(), clusters = clusters.len()))]
pub(crate) fn write_cluster_credentials(
    dir: &Path,
    clusters: &[ClusterInfo],
) -> Result<Vec<ClusterFiles>> {
    let mut written = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let kubeconfig = dir.join(format!("kubeconfig-{}", cluster.name));
        let talosconfig = dir.join(format!("talosconfig-{}", cluster.name));
        write_secret_file(&kubeconfig, &cluster.kubeconfig, "kubeconfig")?;
        write_secret_file(&talosconfig, &cluster.talosconfig, "talosconfig")?;
        written.push(ClusterFiles {
            name: cluster.name.clone(),
            kubeconfig,
            talosconfig,
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
        });
        let written = write_cluster_credentials(dir.path(), &clusters).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("kubeconfig-edge")).unwrap(),
            "kube-edge"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("talosconfig-k8s")).unwrap(),
            "talos-k8s"
        );
        assert_eq!(written[0].kubeconfig, dir.path().join("kubeconfig-k8s"));
    }
}
