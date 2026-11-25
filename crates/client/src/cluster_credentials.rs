use shared::data::ClusterInfo;
use shared::util::{Emoji, LabFolder};

use crate::lab_files::LabFile;

/// The credentials files of one cluster.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClusterFiles {
    pub name: String,
    pub kubeconfig: LabFile,
    pub talosconfig: LabFile,
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
            cluster.kubeconfig.path.display(),
            cluster.talosconfig.path.display()
        );
        println!(
            "   export KUBECONFIG={} TALOSCONFIG={}",
            cluster.kubeconfig.path.display(),
            cluster.talosconfig.path.display()
        );
        if let Some(gateway) = &cluster.gateway {
            println!("   Gateway: http://{gateway}");
        }
        if let Some(hubble_ui) = &cluster.hubble_ui {
            println!("   Hubble UI: http://{hubble_ui}");
        }
    }
}

/// Each cluster's kubeconfig and talosconfig as its files in `folder`,
/// `.sherpa/k8s/kubeconfig-<cluster>` and `.sherpa/talos/talosconfig-<cluster>`, secrets.
/// Writes nothing.
pub(crate) fn cluster_credential_files(
    folder: &LabFolder,
    clusters: &[ClusterInfo],
) -> Vec<ClusterFiles> {
    clusters
        .iter()
        .map(|cluster| ClusterFiles {
            name: cluster.name.clone(),
            kubeconfig: LabFile {
                path: folder.kubeconfig(&cluster.name),
                content: cluster.kubeconfig.clone(),
                secret: Some("kubeconfig"),
            },
            talosconfig: LabFile {
                path: folder.talosconfig(&cluster.name),
                content: cluster.talosconfig.clone(),
                secret: Some("talosconfig"),
            },
            gateway: cluster.gateway.clone(),
            hubble_ui: cluster.hubble_ui.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    use crate::lab_files::write_lab_file;

    #[test]
    fn lab_credentials_005_each_cluster_s_files_are_saved_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let clusters = ["k8s", "edge"].map(|name| ClusterInfo {
            name: name.to_owned(),
            endpoint: format!("https://{name}:6443"),
            talosconfig: format!("talos-{name}"),
            kubeconfig: format!("kube-{name}"),
            gateway: None,
            hubble_ui: None,
        });
        let folder = LabFolder::new(dir.path());
        let written = cluster_credential_files(&folder, &clusters);
        assert_eq!(written.len(), 2);
        for cluster in &written {
            write_lab_file(&folder, &cluster.kubeconfig).unwrap();
            write_lab_file(&folder, &cluster.talosconfig).unwrap();
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(".sherpa/k8s/kubeconfig-edge")).unwrap(),
            "kube-edge"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(".sherpa/talos/talosconfig-k8s")).unwrap(),
            "talos-k8s"
        );
        assert_eq!(
            written[0].kubeconfig.path,
            dir.path().join(".sherpa/k8s/kubeconfig-k8s")
        );
    }
}
