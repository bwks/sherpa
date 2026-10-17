use std::collections::HashSet;

use anyhow::{Result, bail};

use shared::data::{ClusterRole, NodeModel};
use shared::konst::TALOS_VIP_HOST_OFFSETS;
use topology::Manifest;

/// Longest cluster name; cluster names are used in DNS names and certificate SANs.
const MAX_CLUSTER_NAME_LEN: usize = 63;

/// Validate the manifest's Kubernetes clusters and the nodes that join them.
///
/// - cluster names are unique, lowercase DNS labels;
/// - every cluster has at least one node and at least one control plane;
/// - a node names a defined cluster and a role together, and only Talos Linux nodes join
///   clusters;
/// - clustered nodes do not set `ztp_config`, because Sherpa generates their machine configs;
/// - there are no more clusters with several control planes than shared VIP addresses.
pub fn validate_clusters(manifest: &Manifest) -> Result<()> {
    let clusters = manifest.clusters.as_deref().unwrap_or_default();
    let mut names = HashSet::new();
    for cluster in clusters {
        validate_cluster_name(&cluster.name)?;
        if !names.insert(cluster.name.as_str()) {
            bail!(
                "Manifest - cluster '{}' is defined more than once",
                cluster.name
            );
        }
    }

    for node in &manifest.nodes {
        match (&node.cluster, node.cluster_role) {
            (None, None) => continue,
            (Some(cluster), None) => bail!(
                "Manifest - node '{}' joins cluster '{cluster}' but has no cluster_role",
                node.name
            ),
            (None, Some(role)) => bail!(
                "Manifest - node '{}' has cluster_role '{role}' but no cluster",
                node.name
            ),
            (Some(cluster), Some(_)) => {
                if !names.contains(cluster.as_str()) {
                    bail!(
                        "Manifest - node '{}' joins cluster '{cluster}', which is not defined in [[clusters]]",
                        node.name
                    );
                }
                if node.model != NodeModel::TalosLinux {
                    bail!(
                        "Manifest - node '{}' is a {} node; only talos_linux nodes can join a cluster",
                        node.name,
                        node.model
                    );
                }
                if node.ztp_config.is_some() {
                    bail!(
                        "Manifest - node '{}' sets ztp_config but joins cluster '{cluster}'; Sherpa generates the machine configs of cluster nodes",
                        node.name
                    );
                }
            }
        }
    }

    let mut ha_clusters = 0;
    for cluster in clusters {
        let mut members = 0;
        let mut control_planes = 0;
        for node in &manifest.nodes {
            if node.cluster.as_deref() == Some(cluster.name.as_str()) {
                members += 1;
                if node.cluster_role == Some(ClusterRole::ControlPlane) {
                    control_planes += 1;
                }
            }
        }
        if members == 0 {
            bail!("Manifest - cluster '{}' has no nodes", cluster.name);
        }
        if control_planes == 0 {
            bail!(
                "Manifest - cluster '{}' has no control_plane node",
                cluster.name
            );
        }
        if control_planes > 1 {
            ha_clusters += 1;
        }
    }
    let vip_count = TALOS_VIP_HOST_OFFSETS.clone().count();
    if ha_clusters > vip_count {
        bail!(
            "Manifest - {ha_clusters} clusters have more than one control_plane, but a lab has only {vip_count} shared cluster addresses"
        );
    }
    Ok(())
}

fn validate_cluster_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= MAX_CLUSTER_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !valid {
        bail!(
            "Manifest - cluster name '{name}' must be 1-{MAX_CLUSTER_NAME_LEN} lowercase letters, digits or '-', and not start or end with '-'"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
name = "lab"
clusters = [{ name = "k8s" }]
nodes = [
  { name = "cp1", model = "talos_linux", cluster = "k8s", cluster_role = "control_plane" },
  { name = "w1", model = "talos_linux", cluster = "k8s", cluster_role = "worker" },
  { name = "dev", model = "ubuntu_linux" },
]
"#;

    fn manifest(toml: &str) -> Manifest {
        toml::from_str(toml).unwrap()
    }

    fn error(toml: &str) -> String {
        validate_clusters(&manifest(toml)).unwrap_err().to_string()
    }

    #[test]
    fn a_cluster_with_a_control_plane_and_workers_is_valid() {
        validate_clusters(&manifest(VALID)).unwrap();
    }

    #[test]
    fn a_manifest_without_clusters_is_valid() {
        validate_clusters(&manifest(
            r#"
name = "lab"
nodes = [{ name = "talos", model = "talos_linux" }]
"#,
        ))
        .unwrap();
    }

    #[test]
    fn cluster_names_must_be_unique_dns_labels() {
        for name in ["K8s", "-k8s", "k8s-", "k_8s", ""] {
            let toml = VALID.replace(r#"{ name = "k8s" }"#, &format!(r#"{{ name = "{name}" }}"#));
            assert!(validate_clusters(&manifest(&toml)).is_err(), "{name:?}");
        }
        let toml = VALID.replace(
            r#"clusters = [{ name = "k8s" }]"#,
            r#"clusters = [{ name = "k8s" }, { name = "k8s" }]"#,
        );
        assert!(error(&toml).contains("defined more than once"));
    }

    #[test]
    fn nodes_need_both_a_defined_cluster_and_a_role() {
        let toml = VALID.replace(r#", cluster_role = "worker""#, "");
        assert!(error(&toml).contains("has no cluster_role"));
        let toml = VALID.replace(
            r#"cluster = "k8s", cluster_role = "worker""#,
            r#"cluster_role = "worker""#,
        );
        assert!(error(&toml).contains("but no cluster"));
        let toml = VALID.replace(
            r#"name = "w1", model = "talos_linux", cluster = "k8s""#,
            r#"name = "w1", model = "talos_linux", cluster = "other""#,
        );
        assert!(error(&toml).contains("not defined"));
    }

    #[test]
    fn only_talos_nodes_without_their_own_config_join_clusters() {
        let toml = VALID.replace(
            r#"name = "w1", model = "talos_linux""#,
            r#"name = "w1", model = "ubuntu_linux""#,
        );
        assert!(error(&toml).contains("only talos_linux nodes"));
        let toml = VALID.replace(
            r#"name = "w1", model = "talos_linux""#,
            r#"name = "w1", model = "talos_linux", ztp_config = "w1.yaml""#,
        );
        assert!(error(&toml).contains("sets ztp_config"));
    }

    #[test]
    fn clusters_need_nodes_and_a_control_plane() {
        let toml = VALID.replace(
            r#"clusters = [{ name = "k8s" }]"#,
            r#"clusters = [{ name = "k8s" }, { name = "empty" }]"#,
        );
        assert!(error(&toml).contains("'empty' has no nodes"));
        let toml = VALID.replace("control_plane", "worker");
        assert!(error(&toml).contains("has no control_plane"));
    }

    #[test]
    fn highly_available_clusters_are_limited_by_shared_addresses() {
        let count = TALOS_VIP_HOST_OFFSETS.clone().count();
        let mut clusters = vec![];
        let mut nodes = vec![];
        for index in 0..=count {
            clusters.push(format!(r#"{{ name = "c{index}" }}"#));
            for member in 0..2 {
                nodes.push(format!(
                    r#"{{ name = "c{index}-cp{member}", model = "talos_linux", cluster = "c{index}", cluster_role = "control_plane" }}"#
                ));
            }
        }
        let toml = format!(
            "name = \"lab\"\nclusters = [{}]\nnodes = [{}]\n",
            clusters.join(", "),
            nodes.join(", ")
        );
        assert!(error(&toml).contains("shared cluster addresses"));
    }
}
