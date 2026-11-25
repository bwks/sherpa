use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::{Manifest, Node, TopologyError};
use shared::data::{ClusterCni, NodeModel};
use shared::konst::{
    TALOS_LOAD_BALANCER_BLOCK_SIZE, TALOS_LOAD_BALANCER_HOST_OFFSETS, TALOS_VIP_HOST_OFFSETS,
};

/// Longest cluster name; cluster names are used in DNS names and certificate SANs.
const MAX_CLUSTER_NAME_LEN: usize = 63;
/// Controller counts a cluster may have. etcd needs a majority of members, so two
/// controllers tolerate no failure and are refused; three tolerate one.
const ALLOWED_CONTROLLER_COUNTS: [usize; 2] = [1, 3];

/// Validate the manifest's Talos Kubernetes clusters.
///
/// - cluster names are unique, lowercase DNS labels;
/// - every cluster has one or three controllers;
/// - every member is a defined `talos_linux` node without `ztp.config` (Sherpa generates
///   member machine configs), listed once across all clusters;
/// - `oidc` names a defined `keycloak_server` node, and `vault` a defined `hashicorp_vault` node
///   without `ztp.config` (Sherpa configures only the Vault it initialises);
/// - there are no more clusters with three controllers than shared VIP addresses;
/// - there are no more Cilium clusters than blocks of load balancer addresses.
///
/// # Errors
///
/// Fails on the first rule a cluster breaks.
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
    let mut cilium_clusters = 0;
    for cluster in clusters {
        validate_cluster_name(&cluster.name)?;
        if !names.insert(cluster.name.as_str()) {
            bail!(TopologyError::DuplicateCluster {
                cluster: cluster.name.clone(),
            });
        }
        let controllers = cluster.controllers.len();
        if !ALLOWED_CONTROLLER_COUNTS.contains(&controllers) {
            bail!(TopologyError::ControllerCount {
                cluster: cluster.name.clone(),
                controllers,
            });
        }
        if controllers > 1 {
            ha_clusters += 1;
        }
        if cluster.cni == ClusterCni::Cilium {
            cilium_clusters += 1;
        }
        for (member, _) in cluster.members() {
            let Some(node) = nodes.get(member) else {
                bail!(TopologyError::UnknownClusterMember {
                    cluster: cluster.name.clone(),
                    node: member.to_owned(),
                });
            };
            if node.model != NodeModel::TalosLinux {
                bail!(TopologyError::ClusterMemberNotTalos {
                    cluster: cluster.name.clone(),
                    node: member.to_owned(),
                    model: node.model,
                });
            }
            if node.ztp.config.is_some() {
                bail!(TopologyError::ClusterMemberOwnZtpConfig {
                    node: member.to_owned(),
                    cluster: cluster.name.clone(),
                });
            }
            if !members.insert(member) {
                bail!(TopologyError::ClusterMemberRepeated {
                    node: member.to_owned(),
                });
            }
        }
        if let Some(oidc) = &cluster.oidc {
            validate_cluster_service(
                &nodes,
                &cluster.name,
                "oidc",
                oidc,
                NodeModel::KeycloakServer,
            )?;
        }
        if let Some(vault) = &cluster.vault {
            let node = validate_cluster_service(
                &nodes,
                &cluster.name,
                "vault",
                vault,
                NodeModel::HashicorpVault,
            )?;
            if node.ztp.config.is_some() {
                bail!(TopologyError::ClusterVaultOwnZtpConfig {
                    cluster: cluster.name.clone(),
                    vault: vault.clone(),
                });
            }
        }
    }
    let vip_count = TALOS_VIP_HOST_OFFSETS.clone().count();
    if ha_clusters > vip_count {
        bail!(TopologyError::TooManyHaClusters {
            clusters: ha_clusters,
            addresses: vip_count,
        });
    }
    let load_balancer_blocks =
        TALOS_LOAD_BALANCER_HOST_OFFSETS.clone().count() / TALOS_LOAD_BALANCER_BLOCK_SIZE as usize;
    if cilium_clusters > load_balancer_blocks {
        bail!(TopologyError::TooManyCiliumClusters {
            clusters: cilium_clusters,
            blocks: load_balancer_blocks,
        });
    }
    Ok(())
}

/// Check that a cluster's `field` names a defined node of `model`, and return the node.
fn validate_cluster_service<'a>(
    nodes: &HashMap<&str, &'a Node>,
    cluster_name: &str,
    field: &'static str,
    node_name: &str,
    model: NodeModel,
) -> Result<&'a Node> {
    let Some(node) = nodes.get(node_name) else {
        bail!(TopologyError::UnknownClusterService {
            cluster: cluster_name.to_owned(),
            field,
            node: node_name.to_owned(),
        });
    };
    if node.model != model {
        bail!(TopologyError::ClusterServiceModel {
            cluster: cluster_name.to_owned(),
            field,
            node: node_name.to_owned(),
            model: node.model,
            expected: model,
        });
    }
    Ok(node)
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
  { name = "kc01", model = "keycloak_server" },
  { name = "vault01", model = "hashicorp_vault" },
]
"#;
    const CLUSTER: &str = r#"{ name = "k8s", controllers = ["cp1"], workers = ["w1"] }"#;

    fn manifest(toml: &str) -> Manifest {
        toml::from_str(toml).unwrap()
    }

    fn error(toml: &str) -> TopologyError {
        validate_talos_clusters(&manifest(toml))
            .unwrap_err()
            .downcast::<TopologyError>()
            .expect("a topology error")
    }

    /// `VALID` with its cluster definition replaced.
    fn with_cluster(cluster: &str) -> String {
        VALID.replace(CLUSTER, cluster)
    }

    #[test]
    fn manifest_clusters_002_a_cluster_name_that_is_not_a_lowercase_dns_label_is_refused() {
        for name in ["K8s", "-k8s", "k8s-", "k_8s", ""] {
            let toml = with_cluster(&format!(r#"{{ name = "{name}", controllers = ["cp1"] }}"#));
            assert!(
                validate_talos_clusters(&manifest(&toml)).is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn manifest_clusters_003_two_clusters_with_one_name_are_refused_naming_it() {
        let toml = with_cluster(
            r#"{ name = "k8s", controllers = ["cp1"] }, { name = "k8s", controllers = ["cp2"] }"#,
        );
        assert_eq!(
            error(&toml),
            TopologyError::DuplicateCluster {
                cluster: "k8s".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_clusters_004_a_cluster_with_other_than_one_or_three_controllers_is_refused() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1", "cp2", "cp3"] }"#);
        validate_talos_clusters(&manifest(&toml)).unwrap();
        for (controllers, count) in [
            ("[]", 0),
            (r#"["cp1", "cp2"]"#, 2),
            (r#"["cp1", "cp2", "cp3", "cp4"]"#, 4),
        ] {
            let toml = with_cluster(&format!(
                r#"{{ name = "k8s", controllers = {controllers} }}"#
            ));
            assert_eq!(
                error(&toml),
                TopologyError::ControllerCount {
                    cluster: "k8s".to_owned(),
                    controllers: count,
                }
            );
        }
    }

    #[test]
    fn manifest_clusters_005_a_member_that_is_not_a_node_of_the_manifest_is_refused() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], workers = ["ghost"] }"#);
        assert_eq!(
            error(&toml),
            TopologyError::UnknownClusterMember {
                cluster: "k8s".to_owned(),
                node: "ghost".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_clusters_006_a_member_that_is_not_a_talos_linux_node_is_refused() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], workers = ["dev"] }"#);
        assert_eq!(
            error(&toml),
            TopologyError::ClusterMemberNotTalos {
                cluster: "k8s".to_owned(),
                node: "dev".to_owned(),
                model: NodeModel::UbuntuLinux,
            }
        );
    }

    #[test]
    fn machine_config_007_a_cluster_member_with_its_own_ztp_config_is_refused() {
        let toml = VALID.replace(
            r#"{ name = "w1", model = "talos_linux" }"#,
            r#"{ name = "w1", model = "talos_linux", ztp = { config = "w1.yaml" } }"#,
        );
        assert_eq!(
            error(&toml),
            TopologyError::ClusterMemberOwnZtpConfig {
                node: "w1".to_owned(),
                cluster: "k8s".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_clusters_007_a_node_listed_twice_is_refused_naming_it() {
        for (cluster, node) in [
            (
                r#"{ name = "k8s", controllers = ["cp1"], workers = ["w1", "w1"] }"#,
                "w1",
            ),
            (
                r#"{ name = "k8s", controllers = ["cp1"], workers = ["cp1"] }"#,
                "cp1",
            ),
            (
                r#"{ name = "a", controllers = ["cp1"] }, { name = "b", controllers = ["cp1"] }"#,
                "cp1",
            ),
        ] {
            assert_eq!(
                error(&with_cluster(cluster)),
                TopologyError::ClusterMemberRepeated {
                    node: node.to_owned(),
                }
            );
        }
    }

    #[test]
    fn manifest_clusters_008_a_third_cluster_with_three_controllers_is_refused() {
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
            r#"
            name = "lab"
            talos_clusters = [{}]
            nodes = [{}]
            "#,
            clusters.join(", "),
            nodes.join(", ")
        );
        assert_eq!(
            error(&toml),
            TopologyError::TooManyHaClusters {
                clusters: count + 1,
                addresses: count,
            }
        );
    }

    /// A manifest with `count` single-controller Cilium clusters.
    fn cilium_clusters(count: usize) -> String {
        let mut clusters = vec![];
        let mut nodes = vec![];
        for index in 0..count {
            clusters.push(format!(
                r#"{{ name = "c{index}", controllers = ["c{index}-cp"], cni = "cilium" }}"#
            ));
            nodes.push(format!(
                r#"{{ name = "c{index}-cp", model = "talos_linux" }}"#
            ));
        }
        format!(
            r#"
            name = "lab"
            talos_clusters = [{}]
            nodes = [{}]
            "#,
            clusters.join(", "),
            nodes.join(", ")
        )
    }

    #[test]
    fn manifest_clusters_009_two_cilium_clusters_are_taken_and_a_third_is_refused() {
        let blocks = TALOS_LOAD_BALANCER_HOST_OFFSETS.clone().count()
            / TALOS_LOAD_BALANCER_BLOCK_SIZE as usize;
        validate_talos_clusters(&manifest(&cilium_clusters(blocks))).unwrap();
        assert_eq!(
            error(&cilium_clusters(blocks + 1)),
            TopologyError::TooManyCiliumClusters {
                clusters: blocks + 1,
                blocks,
            }
        );
    }

    #[test]
    fn manifest_clusters_010_oidc_and_vault_name_a_keycloak_server_and_a_hashicorp_vault_node() {
        let toml = with_cluster(
            r#"{ name = "k8s", controllers = ["cp1"], oidc = "kc01", vault = "vault01" }"#,
        );
        validate_talos_clusters(&manifest(&toml)).unwrap();

        let missing = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], oidc = "kc02" }"#);
        assert_eq!(
            error(&missing),
            TopologyError::UnknownClusterService {
                cluster: "k8s".to_owned(),
                field: "oidc",
                node: "kc02".to_owned(),
            }
        );
        let wrong_model =
            with_cluster(r#"{ name = "k8s", controllers = ["cp1"], oidc = "vault01" }"#);
        assert_eq!(
            error(&wrong_model),
            TopologyError::ClusterServiceModel {
                cluster: "k8s".to_owned(),
                field: "oidc",
                node: "vault01".to_owned(),
                model: NodeModel::HashicorpVault,
                expected: NodeModel::KeycloakServer,
            }
        );
        let wrong_model =
            with_cluster(r#"{ name = "k8s", controllers = ["cp1"], vault = "kc01" }"#);
        assert_eq!(
            error(&wrong_model),
            TopologyError::ClusterServiceModel {
                cluster: "k8s".to_owned(),
                field: "vault",
                node: "kc01".to_owned(),
                model: NodeModel::KeycloakServer,
                expected: NodeModel::HashicorpVault,
            }
        );
    }

    #[test]
    fn machine_config_008_a_cluster_vault_with_its_own_ztp_config_is_refused() {
        let toml = with_cluster(r#"{ name = "k8s", controllers = ["cp1"], vault = "vault01" }"#)
            .replace(
                r#"{ name = "vault01", model = "hashicorp_vault" }"#,
                r#"{ name = "vault01", model = "hashicorp_vault", ztp = { config = "dWk=" } }"#,
            );
        assert_eq!(
            error(&toml),
            TopologyError::ClusterVaultOwnZtpConfig {
                cluster: "k8s".to_owned(),
                vault: "vault01".to_owned(),
            }
        );
    }
}
