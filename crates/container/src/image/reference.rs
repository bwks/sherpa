//! The image reference Sherpa pulls, checks and creates containers from.

/// The reference of image `repo` at `tag`, kept as written. Docker resolves a short Docker Hub
/// name such as `nats` to `docker.io/library/nats` itself, so the same reference pulls the
/// image, finds it and creates containers from it.
#[must_use]
pub fn image_reference(repo: &str, tag: &str) -> String {
    format!("{repo}:{tag}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_official_docker_hub_image_keeps_its_full_name() {
        assert_eq!(
            image_reference("docker.io/library/nats", "2.11"),
            "docker.io/library/nats:2.11"
        );
    }

    #[test]
    fn an_official_docker_hub_image_keeps_its_short_name() {
        assert_eq!(image_reference("nats", "2.11"), "nats:2.11");
    }

    #[test]
    fn a_third_party_registry_image_keeps_its_path() {
        assert_eq!(
            image_reference("ghcr.io/nokia/srlinux", "24.10.1"),
            "ghcr.io/nokia/srlinux:24.10.1"
        );
        assert_eq!(
            image_reference("localhost:5000/team/img", "1"),
            "localhost:5000/team/img:1"
        );
    }
}
