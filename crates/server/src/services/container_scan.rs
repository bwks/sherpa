//! Matching the images in local Docker to container models, for an image scan.
//!
//! Pure: the Docker listing is passed in, so the matching is tested without a daemon.

use shared::data::{NodeConfig, NodeKind, NodeModel, SkippedModel};

use crate::validate::validate_image_version;

/// Why a container model with no built-in repo is skipped.
pub(crate) const NO_BUILT_IN_REPO: &str = "no built-in repo";

/// The Docker Hub registry names Docker drops when it lists an image.
const DOCKER_HUB_PREFIXES: [&str; 2] = ["docker.io/", "index.docker.io/"];

/// The namespace of Docker Hub's official images, which Docker also drops.
const DOCKER_HUB_OFFICIAL: &str = "library/";

/// A local Docker image to record as a version of a container model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerImageMatch {
    pub model: NodeModel,
    /// The repo the version is recorded with, and that its nodes run.
    pub repo: String,
    pub version: String,
}

/// What a container scan matched, and the models it could not match images to.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ContainerScanMatches {
    pub matches: Vec<ContainerImageMatch>,
    pub skipped: Vec<SkippedModel>,
}

/// A container model and the repo whose local tags are its versions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanTarget<'a> {
    pub model: NodeModel,
    pub repo: &'a str,
}

/// Match `local_images`, Docker's `repo:tag` listing, to container models.
///
/// With a `target`, only its repo's tags are matched, as versions of its model. Without one,
/// each container model with a built-in repo gets that repo's tags, and the models with none
/// are skipped. Tags that are not valid image versions are left out.
#[must_use]
pub(crate) fn match_container_images(
    local_images: &[String],
    target: Option<ScanTarget<'_>>,
) -> ContainerScanMatches {
    let tagged: Vec<(&str, &str)> = local_images
        .iter()
        .filter_map(|reference| split_tagged(reference))
        .collect();

    if let Some(target) = target {
        return ContainerScanMatches {
            matches: repo_matches(&tagged, target.model, target.repo),
            skipped: Vec::new(),
        };
    }

    let mut result = ContainerScanMatches::default();
    for model in NodeModel::to_vec() {
        let config = NodeConfig::get_model(model);
        if config.kind != NodeKind::Container {
            continue;
        }
        match config.repo {
            Some(repo) => result.matches.extend(repo_matches(&tagged, model, &repo)),
            None => result.skipped.push(SkippedModel {
                model,
                reason: NO_BUILT_IN_REPO.to_string(),
            }),
        }
    }
    result
}

/// The versions of `model` in `tagged`: the tags of `repo`, recorded with `repo` as given.
fn repo_matches(tagged: &[(&str, &str)], model: NodeModel, repo: &str) -> Vec<ContainerImageMatch> {
    let wanted = docker_listed_name(repo);
    tagged
        .iter()
        .filter(|(local_repo, _)| docker_listed_name(local_repo) == wanted)
        .filter(|(_, tag)| validate_image_version(tag).is_ok())
        .map(|(_, tag)| ContainerImageMatch {
            model,
            repo: repo.to_string(),
            version: (*tag).to_string(),
        })
        .collect()
}

/// The repo and tag of a listed `repo:tag`. The tag follows the last `:` and has no `/`, so a
/// registry port is not taken for one.
fn split_tagged(reference: &str) -> Option<(&str, &str)> {
    reference
        .rsplit_once(':')
        .filter(|(repo, tag)| !repo.is_empty() && !tag.is_empty() && !tag.contains('/'))
}

/// `repo` as Docker lists it: a Docker Hub image without its registry, and an official image
/// without its `library/` namespace.
fn docker_listed_name(repo: &str) -> &str {
    let unregistered = DOCKER_HUB_PREFIXES
        .iter()
        .find_map(|prefix| repo.strip_prefix(prefix))
        .unwrap_or(repo);
    unregistered
        .strip_prefix(DOCKER_HUB_OFFICIAL)
        .unwrap_or(unregistered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images(references: &[&str]) -> Vec<String> {
        references.iter().map(ToString::to_string).collect()
    }

    fn found(model: NodeModel, repo: &str, version: &str) -> ContainerImageMatch {
        ContainerImageMatch {
            model,
            repo: repo.to_string(),
            version: version.to_string(),
        }
    }

    #[test]
    fn built_in_repo_tags_match_their_models() {
        let local = images(&[
            "ghcr.io/nokia/srlinux:24.10.1",
            "localrepo/arista_ceos:4.32.0F",
            "nats:2.11",
            "ubuntu:24.04",
        ]);

        let result = match_container_images(&local, None);

        assert_eq!(
            result.matches,
            [
                found(NodeModel::AristaCeos, "localrepo/arista_ceos", "4.32.0F"),
                found(NodeModel::NokiaSrlinux, "ghcr.io/nokia/srlinux", "24.10.1"),
                found(NodeModel::NatsBus, "nats", "2.11"),
            ]
        );
    }

    #[test]
    fn every_tag_of_a_repo_is_a_version() {
        let local = images(&["nats:2.10", "nats:2.11", "nats:latest"]);

        let versions: Vec<String> = match_container_images(&local, None)
            .matches
            .into_iter()
            .map(|m| m.version)
            .collect();

        assert_eq!(versions, ["2.10", "2.11", "latest"]);
    }

    #[test]
    fn a_repo_is_not_matched_by_its_prefix() {
        let local = images(&[
            "natsio/nats-box:0.14",
            "nats-streaming:0.25",
            "mongo-express:1",
        ]);

        assert_eq!(match_container_images(&local, None).matches, []);
    }

    #[test]
    fn container_models_with_no_built_in_repo_are_skipped() {
        let skipped: Vec<NodeModel> = match_container_images(&images(&["mysql:8.4"]), None)
            .skipped
            .into_iter()
            .map(|s| s.model)
            .collect();

        assert_eq!(
            skipped,
            [
                NodeModel::MysqlDb,
                NodeModel::PostgresqlDb,
                NodeModel::GenericContainer
            ]
        );
    }

    #[test]
    fn a_skipped_model_says_it_has_no_built_in_repo() {
        let result = match_container_images(&[], None);

        assert_eq!(result.skipped.len(), 3);
        assert!(
            result.skipped.iter().all(|s| s.reason == NO_BUILT_IN_REPO),
            "{result:?}"
        );
    }

    #[test]
    fn a_target_matches_only_its_repo_for_its_model() {
        let local = images(&["mysql:8.4", "mysql:9.1", "nats:2.11", "mariadb:11"]);
        let target = ScanTarget {
            model: NodeModel::MysqlDb,
            repo: "mysql",
        };

        let result = match_container_images(&local, Some(target));

        assert_eq!(
            result,
            ContainerScanMatches {
                matches: vec![
                    found(NodeModel::MysqlDb, "mysql", "8.4"),
                    found(NodeModel::MysqlDb, "mysql", "9.1"),
                ],
                skipped: Vec::new(),
            }
        );
    }

    #[test]
    fn a_target_can_give_a_built_in_model_another_repo() {
        let local = images(&["registry.lab:5000/mirror/srlinux:24.10.1"]);
        let target = ScanTarget {
            model: NodeModel::NokiaSrlinux,
            repo: "registry.lab:5000/mirror/srlinux",
        };

        assert_eq!(
            match_container_images(&local, Some(target)).matches,
            [found(
                NodeModel::NokiaSrlinux,
                "registry.lab:5000/mirror/srlinux",
                "24.10.1"
            )]
        );
    }

    #[test]
    fn a_docker_hub_target_matches_the_short_name_docker_lists() {
        let local = images(&["postgres:17", "bitnami/postgresql:17"]);
        for repo in [
            "postgres",
            "docker.io/postgres",
            "docker.io/library/postgres",
        ] {
            let target = ScanTarget {
                model: NodeModel::PostgresqlDb,
                repo,
            };

            assert_eq!(
                match_container_images(&local, Some(target)).matches,
                [found(NodeModel::PostgresqlDb, repo, "17")],
                "{repo}"
            );
        }
    }

    #[test]
    fn untagged_and_invalid_tags_are_left_out() {
        let local = images(&["<none>:<none>", "nats:-bad", "nats:2.11"]);

        assert_eq!(
            match_container_images(&local, None).matches,
            [found(NodeModel::NatsBus, "nats", "2.11")]
        );
    }

    #[test]
    fn a_registry_port_is_not_taken_for_a_tag() {
        let local = images(&["localhost:5000/app:1"]);
        let target = ScanTarget {
            model: NodeModel::GenericContainer,
            repo: "localhost:5000/app",
        };

        assert_eq!(
            match_container_images(&local, Some(target)).matches,
            [found(
                NodeModel::GenericContainer,
                "localhost:5000/app",
                "1"
            )]
        );
    }
}
