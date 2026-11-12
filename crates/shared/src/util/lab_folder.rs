//! Where the Sherpa CLI keeps the files it generates for a lab: a `.sherpa` folder beside the
//! lab's `manifest.toml`, holding `lab-info.toml` and one folder per kind of file.
//!
//! The paths and their modes are listed in `docs/architecture/LAB_FOLDER.md`.

use std::path::{Path, PathBuf};

use crate::konst::{LAB_FILE_NAME, LAB_STATE_DIR, SHERPA_LAB_CA_FILE};

const SSH_DIR: &str = "ssh";
const SSH_CONFIG_FILE: &str = "config";
const SSH_KEY_FILE: &str = "key";
const KNOWN_HOSTS_FILE: &str = "known_hosts";
const CA_DIR: &str = "ca";
const TALOS_DIR: &str = "talos";
const K8S_DIR: &str = "k8s";
const VAULT_DIR: &str = "vault";
const KEYCLOAK_DIR: &str = "keycloak";

/// The paths of the files the Sherpa CLI generates in the lab folder `root`. Building one
/// touches no files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabFolder {
    root: PathBuf,
}

impl LabFolder {
    /// The lab folder `root`. An empty `root` gives paths relative to the current folder,
    /// such as `.sherpa/lab-info.toml`.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// The lab folder whose SSH config is `ssh_config`, when it is a lab's `.sherpa/ssh/config`.
    #[must_use]
    pub fn of_ssh_config(ssh_config: &Path) -> Option<Self> {
        let ssh_dir = ssh_config.parent()?;
        let state_dir = ssh_dir.parent()?;
        let named = |path: &Path, name: &str| path.file_name().is_some_and(|found| found == name);
        (named(ssh_config, SSH_CONFIG_FILE)
            && named(ssh_dir, SSH_DIR)
            && named(state_dir, LAB_STATE_DIR))
        .then(|| Self::new(state_dir.parent().unwrap_or_else(|| Path::new(""))))
    }

    /// The lab folder, which holds `manifest.toml`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `.sherpa`, the folder holding every generated file.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(LAB_STATE_DIR)
    }

    /// `.sherpa/lab-info.toml`.
    #[must_use]
    pub fn lab_info(&self) -> PathBuf {
        self.state_dir().join(LAB_FILE_NAME)
    }

    /// `.sherpa/ssh`.
    #[must_use]
    pub fn ssh_dir(&self) -> PathBuf {
        self.state_dir().join(SSH_DIR)
    }

    /// `.sherpa/ssh/config`, read with `ssh -F`.
    #[must_use]
    pub fn ssh_config(&self) -> PathBuf {
        self.ssh_dir().join(SSH_CONFIG_FILE)
    }

    /// `.sherpa/ssh/key`, the private key the SSH config names.
    #[must_use]
    pub fn ssh_key(&self) -> PathBuf {
        self.ssh_dir().join(SSH_KEY_FILE)
    }

    /// `.sherpa/ssh/known_hosts`, the node host keys the server recorded.
    #[must_use]
    pub fn known_hosts(&self) -> PathBuf {
        self.ssh_dir().join(KNOWN_HOSTS_FILE)
    }

    /// `.sherpa/ca`.
    #[must_use]
    pub fn ca_dir(&self) -> PathBuf {
        self.state_dir().join(CA_DIR)
    }

    /// `.sherpa/ca/lab-ca.crt`.
    #[must_use]
    pub fn lab_ca(&self) -> PathBuf {
        self.ca_dir().join(SHERPA_LAB_CA_FILE)
    }

    /// `.sherpa/talos`.
    #[must_use]
    pub fn talos_dir(&self) -> PathBuf {
        self.state_dir().join(TALOS_DIR)
    }

    /// `.sherpa/talos/talosconfig-<cluster>`.
    #[must_use]
    pub fn talosconfig(&self, cluster: &str) -> PathBuf {
        self.talos_dir().join(talosconfig_file_name(cluster))
    }

    /// `.sherpa/k8s`.
    #[must_use]
    pub fn k8s_dir(&self) -> PathBuf {
        self.state_dir().join(K8S_DIR)
    }

    /// `.sherpa/k8s/kubeconfig-<cluster>`.
    #[must_use]
    pub fn kubeconfig(&self, cluster: &str) -> PathBuf {
        self.k8s_dir().join(kubeconfig_file_name(cluster))
    }

    /// `.sherpa/vault`.
    #[must_use]
    pub fn vault_dir(&self) -> PathBuf {
        self.state_dir().join(VAULT_DIR)
    }

    /// `.sherpa/vault/vault-<node>.toml`.
    #[must_use]
    pub fn vault(&self, node: &str) -> PathBuf {
        self.vault_dir().join(vault_file_name(node))
    }

    /// `.sherpa/keycloak`.
    #[must_use]
    pub fn keycloak_dir(&self) -> PathBuf {
        self.state_dir().join(KEYCLOAK_DIR)
    }

    /// `.sherpa/keycloak/keycloak-<node>.toml`.
    #[must_use]
    pub fn keycloak(&self, node: &str) -> PathBuf {
        self.keycloak_dir().join(keycloak_file_name(node))
    }
}

/// The name of a cluster's talosconfig: `talosconfig-<cluster>`.
#[must_use]
pub fn talosconfig_file_name(cluster: &str) -> String {
    format!("talosconfig-{cluster}")
}

/// The name of a cluster's kubeconfig: `kubeconfig-<cluster>`.
#[must_use]
pub fn kubeconfig_file_name(cluster: &str) -> String {
    format!("kubeconfig-{cluster}")
}

/// The name of a Vault node's credentials file: `vault-<node>.toml`.
#[must_use]
pub fn vault_file_name(node: &str) -> String {
    format!("vault-{node}.toml")
}

/// The name of a Keycloak node's users file: `keycloak-<node>.toml`.
#[must_use]
pub fn keycloak_file_name(node: &str) -> String {
    format!("keycloak-{node}.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_under_the_state_folder() {
        let folder = LabFolder::new(Path::new("/home/me/lab"));
        let state = Path::new("/home/me/lab/.sherpa");
        assert_eq!(folder.state_dir(), state);
        assert_eq!(folder.lab_info(), state.join("lab-info.toml"));
        assert_eq!(folder.ssh_config(), state.join("ssh/config"));
        assert_eq!(folder.ssh_key(), state.join("ssh/key"));
        assert_eq!(folder.known_hosts(), state.join("ssh/known_hosts"));
        assert_eq!(folder.lab_ca(), state.join("ca/lab-ca.crt"));
        assert_eq!(
            folder.talosconfig("k8s"),
            state.join("talos/talosconfig-k8s")
        );
        assert_eq!(folder.kubeconfig("k8s"), state.join("k8s/kubeconfig-k8s"));
        assert_eq!(folder.vault("v1"), state.join("vault/vault-v1.toml"));
        assert_eq!(
            folder.keycloak("kc"),
            state.join("keycloak/keycloak-kc.toml")
        );
    }

    #[test]
    fn an_empty_root_gives_paths_relative_to_the_current_folder() {
        let folder = LabFolder::new(Path::new(""));
        assert_eq!(folder.lab_info(), Path::new(".sherpa/lab-info.toml"));
        assert_eq!(folder.ssh_config(), Path::new(".sherpa/ssh/config"));
    }

    #[test]
    fn the_lab_folder_is_found_from_its_ssh_config() {
        let folder = LabFolder::new(Path::new("/home/me/lab"));
        assert_eq!(LabFolder::of_ssh_config(&folder.ssh_config()), Some(folder));
        for other in [
            "/home/me/lab/sherpa_ssh_config",
            "/home/me/lab/.sherpa/config",
            "/home/me/lab/.sherpa/ssh/key",
            "/home/me/lab/other/ssh/config",
            "config",
        ] {
            assert_eq!(LabFolder::of_ssh_config(Path::new(other)), None, "{other}");
        }
    }
}
