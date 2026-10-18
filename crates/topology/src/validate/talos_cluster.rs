use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::{Manifest, Node};
use shared::data::NodeModel;
use shared::konst::TALOS_VIP_HOST_OFFSETS;

/// Longest cluster name; cluster names are used in DNS names and certificate SANs.
const MAX_CLUSTER_NAME_LEN: usize = 63;
/// Controller counts a cluster may have. etcd needs a majority of members, so two
/// controllers tolerate no failure and are refused; three tolerate one.
const ALLOWED_CONTROLLER_COUNTS: [usize; 2] = [1, 3];

/// Validate the manifest's Talos Kubernetes clusters.
///
/// - cluster names are unique, lowercase DNS labels;
/// - every cluster has one or three controllers;
/// - every member is a defined `talos_linux` node without `ztp_config` (Sherpa generates
///   member machine configs), listed once across all clusters;
/// - there are no more clusters with three controllers than shared VIP addresses.
pub fn validate_talos_clusters(manifest: &Manifest) -> Result<()> {
    let clusters = manifest.talos_clusters.as_deref().unwrap_or_default();
    let nodes: HashMap<&str, &Node> = manifest
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let mut names = HashSet::new();
    let mut members = HashSet::new();
    let mut ha_clusters = 0;
    for cluster in clusters {
        validate_cluster_name(&cluster.name)?;
        if !names.insert(cluster.name.as_str()) {
            bail!(
                "Manifest - talos cluster '{}' is defined more than once",
                cluster.name
            );
        }
        let controllers = cluster.controllers.len();
        if !ALLOWED_CONTROLLER_COUNTS.contains(&controllers) {
            bail!(
                "Manifest - talos cluster '{}' has {controllers} controllers; a cluster needs 1 or 3, because etcd needs a majority of controllers to stay up",
                cluster.name
            );
        }
        if controllers > 1 {
            ha_clusters += 1;
        }
        for (member, _) in cluster.members() {
            let Some(node) = nodes.get(member) else {
                bail!(
                    "Manifest - talos cluster '{}' lists node '{member}', which is not defined in nodes",
                    cluster.name
                );
            };
            if node.model != NodeModel::TalosLinux {
                bail!(
                    "Manifest - talos cluster '{}' lists node '{member}', a {} node; only talos_linux nodes can join a cluster",
                    cluster.name,
                    node.model
                );
            }
            if node.ztp_config.is_some() {
                bail!(
                    "Manifest - node '{member}' sets ztp_config but joins talos cluster '{}'; Sherpa generates the machine configs of cluster nodes",
                    cluster.name
                );
            }
            if !members.insert(member) {
                bail!(
                    "Manifest - node '{member}' is listed more than once in talos_clusters; a node joins one cluster, as a controller or a worker"
                );
            }
        }
    }
    let vip_count = TALOS_VIP_HOST_OFFSETS.clone().count();
    if ha_clusters > vip_count {
        bail!(
            "Manifest - {ha_clusters} talos clusters have 3 controllers, but a lab has only {vip_count} shared cluster addresses"
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
            "Manifest - talos cluster name '{name}' must be 1-{MAX_CLUSTER_NAME_LEN} lowercase letters, digits or '-', and not start or end with '-'"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
name = "lab"
talos_clusters = [{ name = "k8s", controllers = ["cp1"], workers = ["w1"] }]
nodes = [
  { name = "cp1", model = "talos_linux" },
  { name = "cp2", model = "talos_linux" },
  { name = "cp3", model = "talos_linux" },
  { name = "cp4", model = "talos_linux" },
  { name = "w1", model = "talos_linux" },
  { name = "dev", model = "ubuntu_linux" },
]
"#;
    const CLUSTER: &str = r#"{ name = "k8s", controllers = ["cp1"], workers = ["w1"] }"#;

    fn manifest(toml: &str) -> Manifest {
        toml::from_str(toml).unwrap()
    }

    fn error(toml: &str) -> String {
        validate_talos_clusters(&manifest(toml))
            .unwrap_err()
            .to_string()
    }

    /// `VALID` with its cluster definition replaced.
    fn with_cluster(cluster: &str) -> String {
        VALID.replace(CLUSTER, cluster)
    }

    #[test]
    fn a_cluster_with_a_controller_and_workers_is_valid() {
        validate_talos_clusters(&manifest(VALID)).unwrap();
    }

    #[test]
    fn a_cluster_without_workers_is_valid() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"] }"#);
        validate_talos_clusters(&manifest(&toml)).unwrap();
    }

    #[test]
    fn a_manifest_without_clusters_is_valid() {
        validate_talos_clusters(&manifest(
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
            let toml = with_cluster(&format!(r#"{{ name = "{name}", controllers = ["cp1"] }}"#));
            assert!(
                validate_talos_clusters(&manifest(&toml)).is_err(),
                "{name:?}"
            );
        }
        let toml = with_cluster(
            r#"{ name = "k8s", controllers = ["cp1"] }, { name = "k8s", controllers = ["cp2"] }"#,
        );
        assert!(error(&toml).contains("defined more than once"));
    }

    #[test]
    fn clusters_have_one_or_three_controllers() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1", "cp2", "cp3"] }"#);
        validate_talos_clusters(&manifest(&toml)).unwrap();
        for controllers in ["[]", r#"["cp1", "cp2"]"#, r#"["cp1", "cp2", "cp3", "cp4"]"#] {
            let toml = with_cluster(&format!(
                r#"{{ name = "k8s", controllers = {controllers} }}"#
            ));
            let message = error(&toml);
            assert!(message.contains("'k8s' has"), "{message}");
            assert!(message.contains("needs 1 or 3"), "{message}");
        }
    }

    #[test]
    fn members_must_be_defined_nodes() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], workers = ["ghost"] }"#);
        assert!(error(&toml).contains("lists node 'ghost', which is not defined"));
    }

    #[test]
    fn only_talos_nodes_without_their_own_config_join_clusters() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], workers = ["dev"] }"#);
        assert!(error(&toml).contains("only talos_linux nodes"));
        let toml = VALID.replace(
            r#"{ name = "w1", model = "talos_linux" }"#,
            r#"{ name = "w1", model = "talos_linux", ztp_config = "w1.yaml" }"#,
        );
        assert!(error(&toml).contains("sets ztp_config"));
    }

    #[test]
    fn a_node_joins_one_cluster_once() {
        for cluster in [
            r#"{ name = "k8s", controllers = ["cp1"], workers = ["w1", "w1"] }"#,
            r#"{ name = "k8s", controllers = ["cp1"], workers = ["cp1"] }"#,
            r#"{ name = "a", controllers = ["cp1"] }, { name = "b", controllers = ["cp1"] }"#,
        ] {
            let message = error(&with_cluster(cluster));
            assert!(message.contains("listed more than once"), "{message}");
        }
    }

    #[test]
    fn highly_available_clusters_are_limited_by_shared_addresses() {
        let count = TALOS_VIP_HOST_OFFSETS.clone().count();
        let mut clusters = vec![];
        let mut nodes = vec![];
        for index in 0..=count {
            let mut controllers = vec![];
            for member in 0..3 {
                controllers.push(format!(r#""c{index}-cp{member}""#));
                nodes.push(format!(
                    r#"{{ name = "c{index}-cp{member}", model = "talos_linux" }}"#
                ));
            }
            clusters.push(format!(
                r#"{{ name = "c{index}", controllers = [{}] }}"#,
                controllers.join(", ")
            ));
        }
        let toml = format!(
            "name = \"lab\"\ntalos_clusters = [{}]\nnodes = [{}]\n",
            clusters.join(", "),
            nodes.join(", ")
        );
        assert!(error(&toml).contains("shared cluster addresses"));
    }
}
