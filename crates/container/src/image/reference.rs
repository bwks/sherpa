//! The image reference Sherpa pulls, checks and creates containers from.

/// The reference of image `repo` at `tag`, kept as written. Docker resolves a short Docker Hub
/// name such as `nats` to `docker.io/library/nats` itself, so the same reference pulls the
/// image, finds it and creates containers from it.
#[must_use]
pub fn image_reference(repo: &str, tag: &str) -> String {
    format!("{repo}:{tag}")
}
