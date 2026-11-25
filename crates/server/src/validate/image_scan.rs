//! Checks on an image scan request.

use anyhow::{Result, bail};

use crate::services::error::ServiceError;
use shared::data::{NodeKind, ScanImagesRequest};

/// The longest image repo Docker takes.
const IMAGE_REPO_MAX_LEN: usize = 255;

/// Check an image scan request. A `model` and `repo` come together, name a container model and
/// a repo without a tag, and only scan containers.
///
/// # Errors
///
/// Fails with the `ServiceError` of the rule the request breaks.
pub fn validate_image_scan(request: &ScanImagesRequest) -> Result<()> {
    let (model, repo) = match (request.model, request.repo.as_deref()) {
        (None, None) => return Ok(()),
        (Some(model), Some(repo)) => (model, repo),
        _ => {
            bail!(ServiceError::ScanModelWithoutRepo);
        }
    };
    if request
        .kind
        .as_ref()
        .is_some_and(|kind| *kind != NodeKind::Container)
    {
        bail!(ServiceError::ScanKindNotContainer);
    }
    if !model.kinds().contains(&NodeKind::Container) {
        bail!(ServiceError::ScanModelNotContainer { model });
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
    bail!(ServiceError::ScanRepoInvalid {
        repo: repo.to_owned()
    })
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

    use crate::services::error::ServiceCategory;

    fn request(model: Option<NodeModel>, repo: Option<&str>) -> ScanImagesRequest {
        ScanImagesRequest {
            kind: Some(NodeKind::Container),
            dry_run: false,
            model,
            repo: repo.map(ToString::to_string),
        }
    }

    fn refusal(request: &ScanImagesRequest) -> ServiceError {
        let error = validate_image_scan(request).unwrap_err();
        let refusal = error.downcast::<ServiceError>().expect("a service error");
        assert_eq!(refusal.category(), Some(ServiceCategory::Invalid));
        refusal
    }

    #[test]
    fn image_scan_009_model_and_repo_are_refused_unless_both_are_given() {
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
            let request = request(Some(NodeModel::Generic), Some(repo));
            assert!(validate_image_scan(&request).is_ok(), "{repo}");
        }
        for request in [
            request(Some(NodeModel::MysqlDb), None),
            request(None, Some("mysql")),
        ] {
            assert_eq!(refusal(&request), ServiceError::ScanModelWithoutRepo);
        }
    }

    #[test]
    fn image_scan_010_model_and_repo_are_refused_for_a_model_or_kind_that_is_not_a_container() {
        assert_eq!(
            refusal(&request(Some(NodeModel::UbuntuLinux), Some("ubuntu"))),
            ServiceError::ScanModelNotContainer {
                model: NodeModel::UbuntuLinux
            }
        );

        for kind in [NodeKind::VirtualMachine, NodeKind::Unikernel] {
            let request = ScanImagesRequest {
                kind: Some(kind),
                ..request(Some(NodeModel::MysqlDb), Some("mysql"))
            };
            assert_eq!(refusal(&request), ServiceError::ScanKindNotContainer);
        }

        let without_kind = ScanImagesRequest {
            kind: None,
            ..request(Some(NodeModel::MysqlDb), Some("mysql"))
        };
        assert!(validate_image_scan(&without_kind).is_ok());
    }

    #[test]
    fn image_scan_011_a_repo_that_is_not_a_valid_repository_name_is_refused() {
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
            assert_eq!(
                refusal(&request(Some(NodeModel::Generic), Some(repo))),
                ServiceError::ScanRepoInvalid {
                    repo: repo.to_owned()
                },
                "{repo:?}"
            );
        }
    }
}
