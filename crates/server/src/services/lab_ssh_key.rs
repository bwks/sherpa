//! The key pair the `sherpa` account on a lab's nodes logs in with, on nodes that cannot take
//! their owner's own key.
//!
//! `up` makes one per lab, in the lab folder, readable by sherpad only. `destroy` and `clean`
//! delete it with the folder.

use anyhow::{Context, Result};
use ssh_key::Algorithm;
use tracing::instrument;

use shared::konst::{
    LAB_SSH_PRIVATE_KEY_MODE, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE,
};
use shared::util;

/// Where a lab's key pair is on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabSshKeyPaths {
    pub(crate) public: String,
    pub(crate) private: String,
}

impl LabSshKeyPaths {
    /// The key pair of the lab in `lab_dir`.
    pub(crate) fn in_lab(lab_dir: &str) -> Self {
        Self {
            public: format!("{lab_dir}/{SHERPA_SSH_PUBLIC_KEY_FILE}"),
            private: format!("{lab_dir}/{SHERPA_SSH_PRIVATE_KEY_FILE}"),
        }
    }
}

/// Make the lab's own key pair in `lab_dir`, which must exist. It is RSA, as some network
/// operating systems take no other kind, and only sherpad can read the private key.
#[instrument(level = "debug")]
pub(crate) fn create_lab_ssh_key(lab_dir: &str) -> Result<LabSshKeyPaths> {
    util::generate_ssh_keypair(
        lab_dir,
        SHERPA_SSH_PRIVATE_KEY_FILE,
        &Algorithm::Rsa { hash: None },
        LAB_SSH_PRIVATE_KEY_MODE,
    )
    .context("Failed to create the lab's SSH key")?;
    tracing::info!("Created the lab's SSH key");
    Ok(LabSshKeyPaths::in_lab(lab_dir))
}

/// The private key of `paths`, for the lab's owner or an admin. A missing key is an error.
#[instrument(level = "debug")]
pub(crate) async fn read_private_key(paths: &LabSshKeyPaths) -> Result<String> {
    tokio::fs::read_to_string(&paths.private)
        .await
        .with_context(|| {
            format!(
                "The lab's SSH key was not found on the server: {}",
                paths.private
            )
        })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_lab_key_is_in_the_lab_folder() {
        assert_eq!(
            LabSshKeyPaths::in_lab("/opt/sherpa/labs/abcd1234"),
            LabSshKeyPaths {
                public: "/opt/sherpa/labs/abcd1234/sherpa_ssh_key.pub".to_owned(),
                private: "/opt/sherpa/labs/abcd1234/sherpa_ssh_key".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn each_lab_gets_its_own_key_only_sherpad_can_read() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();

        let first_paths = create_lab_ssh_key(first.path().to_str().unwrap()).unwrap();
        let second_paths = create_lab_ssh_key(second.path().to_str().unwrap()).unwrap();

        let mode = std::fs::metadata(&first_paths.private)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let first_key = read_private_key(&first_paths).await.unwrap();
        let second_key = read_private_key(&second_paths).await.unwrap();
        assert!(first_key.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert_ne!(first_key, second_key);
        let user = util::sherpa_user(&first_paths.public).unwrap();
        assert_eq!(user.ssh_public_key.algorithm.to_string(), "ssh-rsa");
    }

    #[tokio::test]
    async fn a_missing_lab_key_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = LabSshKeyPaths::in_lab(dir.path().to_str().unwrap());

        let error = read_private_key(&paths).await.unwrap_err().to_string();

        assert!(error.contains("The lab's SSH key was not found"), "{error}");
        assert!(error.contains(&paths.private), "{error}");
    }

    #[test]
    fn a_lab_key_needs_the_lab_folder() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        assert!(create_lab_ssh_key(missing.to_str().unwrap()).is_err());
    }
}
