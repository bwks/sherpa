//! Planning a container image scan, and reading versions off the tags Docker lists for it.
//!
//! Docker matches local images to a repo (`container::get_local_images` with a reference);
//! this module decides which repos to ask for, for which models, and reads the versions off
//! what Docker returns. It is pure, so it is tested without a daemon.

use shared::data::{NodeConfig, NodeKind, NodeModel, SkippedModel};

use crate::validate::validate_image_version;

/// Why a container model with no built-in repo is skipped.
pub(crate) const NO_BUILT_IN_REPO: &str = "no built-in repo";

/// A container model and the repo whose local tags are its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanTarget {
    pub model: NodeModel,
    /// The repo Docker is asked for, recorded with each version as given.
    pub repo: String,
}

/// The repos a container scan asks Docker for, and the models it cannot ask for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ContainerScanPlan {
    pub targets: Vec<ScanTarget>,
    pub skipped: Vec<SkippedModel>,
}

/// The repos to scan: `target` alone when given, else each container model's built-in repo,
/// with the container models that have none skipped.
#[must_use]
pub(crate) fn plan_container_scan(target: Option<ScanTarget>) -> ContainerScanPlan {
    if let Some(target) = target {
        return ContainerScanPlan {
            targets: vec![target],
            skipped: Vec::new(),
        };
    }

    let mut plan = ContainerScanPlan::default();
    for model in NodeModel::to_vec() {
        let config = NodeConfig::get_model(model);
        if config.kind != NodeKind::Container {
            continue;
        }
        match config.repo {
            Some(repo) => plan.targets.push(ScanTarget { model, repo }),
            None => plan.skipped.push(SkippedModel {
                model,
                reason: NO_BUILT_IN_REPO.to_string(),
            }),
        }
    }
    plan
}

/// The versions in `repo_tags`, the `repo:tag`s Docker matched to one repo: each tag that is a
/// valid image version.
#[must_use]
pub(crate) fn tagged_versions(repo_tags: &[String]) -> Vec<&str> {
    repo_tags
        .iter()
        .filter_map(|repo_tag| listed_tag(repo_tag))
        .filter(|tag| validate_image_version(tag).is_ok())
        .collect()
}

/// The tag of a `repo:tag` Docker listed, which follows the last `:`. Docker has no structured
/// tag field, and lists a repo by its own name for it (`mysql:8.4` for
/// `docker.io/library/mysql`), so the tag is read off the string rather than the repo stripped.
/// An untagged image is listed by digest (`nats@sha256:…`) and has no tag. A registry port is
/// not a valid version, so `localhost:5000/app` is left out by the version check.
fn listed_tag(repo_tag: &str) -> Option<&str> {
    if repo_tag.contains('@') {
        return None;
    }
    repo_tag.rsplit_once(':').map(|(_, tag)| tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(repo_tags: &[&str]) -> Vec<String> {
        repo_tags.iter().map(ToString::to_string).collect()
    }

    fn target(model: NodeModel, repo: &str) -> ScanTarget {
        ScanTarget {
            model,
            repo: repo.to_string(),
        }
    }

    #[test]
    fn a_scan_asks_for_each_built_in_container_repo() {
        let plan = plan_container_scan(None);

        for expected in [
            target(NodeModel::AristaCeos, "localrepo/arista_ceos"),
            target(NodeModel::NokiaSrlinux, "ghcr.io/nokia/srlinux"),
            target(NodeModel::NatsBus, "nats"),
        ] {
            assert!(plan.targets.contains(&expected), "{expected:?}: {plan:?}");
        }
        assert!(
            plan.targets
                .iter()
                .all(|t| NodeConfig::get_model(t.model).kind == NodeKind::Container),
            "{plan:?}"
        );
    }

    #[test]
    fn container_models_with_no_built_in_repo_are_skipped() {
        let plan = plan_container_scan(None);

        let skipped: Vec<NodeModel> = plan.skipped.iter().map(|s| s.model).collect();
        assert_eq!(
            skipped,
            [
                NodeModel::MysqlDb,
                NodeModel::PostgresqlDb,
                NodeModel::GenericContainer
            ]
        );
        assert!(
            plan.skipped.iter().all(|s| s.reason == NO_BUILT_IN_REPO),
            "{plan:?}"
        );
    }

    #[test]
    fn a_target_is_the_only_repo_scanned_and_kept_as_given() {
        let given = target(NodeModel::NokiaSrlinux, "registry.lab:5000/mirror/srlinux");

        assert_eq!(
            plan_container_scan(Some(given.clone())),
            ContainerScanPlan {
                targets: vec![given],
                skipped: Vec::new(),
            }
        );
    }

    #[test]
    fn every_listed_tag_is_a_version() {
        let repo_tags = listed(&["nats:2.10-alpine", "nats:2.11", "nats:latest"]);

        assert_eq!(
            tagged_versions(&repo_tags),
            ["2.10-alpine", "2.11", "latest"]
        );
    }

    #[test]
    fn a_digest_listing_is_not_a_version() {
        let repo_tags = listed(&[
            "nats@sha256:afae2fdbe2e326c65e1246cb83b9d05d319188734f3bd577ac8072daae78a10f",
            "nats:2.11",
        ]);

        assert_eq!(tagged_versions(&repo_tags), ["2.11"]);
    }

    #[test]
    fn untagged_and_invalid_tags_are_left_out() {
        let repo_tags = listed(&["<none>:<none>", "nats:-bad", "nats", "nats:2.11"]);

        assert_eq!(tagged_versions(&repo_tags), ["2.11"]);
    }

    #[test]
    fn a_registry_port_is_not_taken_for_a_tag() {
        let repo_tags = listed(&["localhost:5000/app:1", "localhost:5000/app"]);

        assert_eq!(tagged_versions(&repo_tags), ["1"]);
    }
}
