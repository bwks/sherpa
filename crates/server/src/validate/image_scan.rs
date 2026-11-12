//! Checks on an image scan request.

use anyhow::Result;

use crate::services::error::ServiceError;
use shared::data::{NodeConfig, NodeKind, ScanImagesRequest};

/// The longest image repo Docker takes.
const IMAGE_REPO_MAX_LEN: usize = 255;

/// Check an image scan request. A `model` and `repo` come together, name a container model and
/// a repo without a tag, and only scan containers.
///
/// # Errors
///
/// Fails with `ServiceError::Invalid` naming the rule the request breaks.
pub fn validate_image_scan(request: &ScanImagesRequest) -> Result<()> {
    let (model, repo) = match (request.model, request.repo.as_deref()) {
        (None, None) => return Ok(()),
        (Some(model), Some(repo)) => (model, repo),
        _ => {
            return Err(ServiceError::Invalid(
                "--model and --repo are given together: the repo's local tags are recorded as \
                 versions of the model"
                    .to_string(),
            )
            .into());
        }
    };
    if request
        .kind
        .as_ref()
        .is_some_and(|kind| *kind != NodeKind::Container)
    {
        return Err(ServiceError::Invalid(
            "--model and --repo only scan containers: give them with --container".to_string(),
        )
        .into());
    }
    if NodeConfig::get_model(model).kind != NodeKind::Container {
        return Err(ServiceError::Invalid(format!(
            "'{model}' is not a container model: --model and --repo record local Docker images"
        ))
        .into());
    }
    validate_image_repo(repo)
}

/// Check `repo` is an image repository without a tag or digest: `/`-separated lowercase names
/// of letters, digits, `.`, `_` and `-`, each starting and ending with a letter or digit, after
/// an optional registry host and port such as `ghcr.io` or `localhost:5000`.
fn validate_image_repo(repo: &str) -> Result<()> {
    let components: Vec<&str> = repo.split('/').collect();
    if !repo.contains('@') && components.last().is_some_and(|name| name.contains(':')) {
        return Err(ServiceError::Invalid(format!(
            "Repo '{repo}' has a tag: give the repo without a tag, such as mysql or \
             ghcr.io/nokia/srlinux, and the scan records each of its local tags"
        ))
        .into());
    }
    let (registry, names) = match components.as_slice() {
        [first, rest @ ..] if !rest.is_empty() && is_registry(first) => (Some(*first), rest),
        all => (None, all),
    };
    let valid = repo.len() <= IMAGE_REPO_MAX_LEN
        && registry.is_none_or(is_registry_host)
        && names.iter().all(|name| is_repo_name(name));
    if valid {
        return Ok(());
    }
    Err(ServiceError::Invalid(format!(
        "'{repo}' is not a valid repo: use an image repository without a tag, such as mysql, \
         library/mysql or ghcr.io/nokia/srlinux"
    ))
    .into())
}

/// Whether the first component of a repo names a registry, as Docker reads it.
fn is_registry(component: &str) -> bool {
    component.contains(['.', ':']) || component == "localhost"
}

/// Whether `host` is a registry host with an optional port.
fn is_registry_host(host: &str) -> bool {
    let (name, port) = host.split_once(':').unwrap_or((host, "1"));
    is_name(name, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-')
    }) && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
}

/// Whether `name` is one lowercase component of a repo's path.
fn is_repo_name(name: &str) -> bool {
    is_name(name, |c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
    })
}

/// Whether `name` is made of `allowed` characters and starts and ends with a letter or digit.
fn is_name(name: &str, allowed: impl Fn(char) -> bool) -> bool {
    let ends_well = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    ends_well(name.chars().next()) && ends_well(name.chars().last()) && name.chars().all(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    fn request(model: Option<NodeModel>, repo: Option<&str>) -> ScanImagesRequest {
        ScanImagesRequest {
            kind: Some(NodeKind::Container),
            dry_run: false,
            model,
            repo: repo.map(ToString::to_string),
        }
    }

    fn refusal(request: &ScanImagesRequest) -> String {
        let error = validate_image_scan(request).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::Invalid(_))
            ),
            "{error:#}"
        );
        error.to_string()
    }

    #[test]
    fn a_scan_with_no_model_or_repo_is_accepted_for_every_kind() {
        for kind in [
            None,
            Some(NodeKind::Container),
            Some(NodeKind::VirtualMachine),
            Some(NodeKind::Unikernel),
        ] {
            let request = ScanImagesRequest {
                kind,
                ..request(None, None)
            };
            assert!(validate_image_scan(&request).is_ok(), "{request:?}");
        }
    }

    #[test]
    fn a_container_model_and_repo_are_accepted() {
        for repo in [
            "mysql",
            "library/mysql",
            "docker.io/library/mysql",
            "ghcr.io/nokia/srlinux",
            "localhost:5000/team/app",
            "Registry.Lab:5000/mirror/postgres",
            "quay.io/frrouting/frr",
            "my_org/app-name.v2",
        ] {
            let request = request(Some(NodeModel::GenericContainer), Some(repo));
            assert!(validate_image_scan(&request).is_ok(), "{repo}");
        }
    }

    #[test]
    fn a_model_and_repo_need_no_kind() {
        let request = ScanImagesRequest {
            kind: None,
            ..request(Some(NodeModel::MysqlDb), Some("mysql"))
        };

        assert!(validate_image_scan(&request).is_ok());
    }

    #[test]
    fn a_model_without_a_repo_is_refused() {
        let message = refusal(&request(Some(NodeModel::MysqlDb), None));

        assert!(message.contains("--model and --repo"), "{message}");
    }

    #[test]
    fn a_repo_without_a_model_is_refused() {
        let message = refusal(&request(None, Some("mysql")));

        assert!(message.contains("--model and --repo"), "{message}");
    }

    #[test]
    fn a_model_that_is_not_a_container_is_refused() {
        let message = refusal(&request(Some(NodeModel::UbuntuLinux), Some("ubuntu")));

        assert!(message.contains("not a container model"), "{message}");
    }

    #[test]
    fn a_model_and_repo_with_another_kind_are_refused() {
        for kind in [NodeKind::VirtualMachine, NodeKind::Unikernel] {
            let request = ScanImagesRequest {
                kind: Some(kind),
                ..request(Some(NodeModel::MysqlDb), Some("mysql"))
            };

            assert!(refusal(&request).contains("only scan containers"));
        }
    }

    #[test]
    fn a_repo_with_a_tag_is_refused_with_how_to_write_it() {
        for repo in ["mysql:8.4", "localhost:5000/app:1"] {
            let message = refusal(&request(Some(NodeModel::MysqlDb), Some(repo)));

            assert!(message.contains("without a tag"), "{repo}: {message}");
        }
    }

    #[test]
    fn a_malformed_repo_is_refused() {
        let too_long = "a".repeat(256);
        for repo in [
            "",
            " ",
            "my sql",
            "mysql@sha256:abc",
            "/mysql",
            "mysql/",
            "team//mysql",
            "team/MySQL",
            "MySQL",
            "-mysql",
            "team/.app",
            "my$ql",
            too_long.as_str(),
        ] {
            let message = refusal(&request(Some(NodeModel::GenericContainer), Some(repo)));

            assert!(
                message.contains("is not a valid repo"),
                "{repo:?}: {message}"
            );
        }
    }
}
