//! Vault nodes in the lab lifecycle. Sherpa initialises each Vault node that runs its generated
//! config, keeps the unseal key and root token with the lab's files, and unseals it.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use shared::data::{NodeModel, StatusKind, VaultInfo};
use shared::konst::{READINESS_SLEEP, SHERPA_LABS_PATH};

use super::api::{SealStatus, VaultClient};
use crate::services::progress::ProgressSender;
use crate::tls::generator::write_file_with_permissions;

/// Directory in the lab directory holding each Vault node's credentials.
const VAULT_DIR: &str = "vault";

/// A Vault node's credentials, saved as `<lab>/vault/<node>.toml`, readable only by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredentials {
    pub address: String,
    pub unseal_key: String,
    pub root_token: String,
}

impl VaultCredentials {
    fn into_info(self, name: &str) -> VaultInfo {
        VaultInfo {
            name: name.to_owned(),
            address: self.address,
            root_token: self.root_token,
            unseal_key: self.unseal_key,
        }
    }
}

/// Whether Sherpa initialises and unseals a node: a Vault node running the config Sherpa
/// generates. A user's own config may not serve the API over TLS, so it is left to the user.
pub fn sherpa_initialises_vault(model: &NodeModel, own_config: bool) -> bool {
    *model == NodeModel::HashicorpVault && !own_config
}

/// The lab directory where Sherpa keeps its Vault nodes' credentials.
pub fn vault_dir(lab_id: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH).join(lab_id).join(VAULT_DIR)
}

fn credentials_path(dir: &Path, node_name: &str) -> PathBuf {
    dir.join(format!("{node_name}.toml"))
}

fn save_credentials(dir: &Path, node_name: &str, credentials: &VaultCredentials) -> Result<()> {
    let content = toml::to_string(credentials).context("Failed to serialize Vault credentials")?;
    write_file_with_permissions(&credentials_path(dir, node_name), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save the credentials of Vault node '{node_name}'"))
}

fn load_credentials(dir: &Path, node_name: &str) -> Result<Option<VaultCredentials>> {
    let path = credentials_path(dir, node_name);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let credentials =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(credentials))
}

/// Wait for a Vault node's API, initialise it if it is new, and unseal it if it is sealed.
///
/// A new Vault is initialised with one key share. Its unseal key and root token are saved
/// before it is unsealed, so they are never lost to a failure in between.
#[instrument(skip(lab_ca_pem, progress), fields(%lab_id, %node_name, %node_address))]
pub async fn ensure_vault_unsealed(
    lab_id: &str,
    node_name: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<VaultInfo> {
    let dir = vault_dir(lab_id);
    let client = VaultClient::new(node_address, lab_ca_pem)?;
    let status = wait_for_api(&client, node_name, timeout).await?;

    let credentials = if status.initialized {
        load_credentials(&dir, node_name)?.with_context(|| {
            format!(
                "Vault node '{node_name}' was initialised outside Sherpa, so Sherpa has no unseal key or root token for it"
            )
        })?
    } else {
        let init = client
            .init()
            .await
            .with_context(|| format!("Failed to initialise Vault node '{node_name}'"))?;
        let unseal_key = init
            .keys_base64
            .into_iter()
            .next()
            .with_context(|| format!("Vault node '{node_name}' returned no unseal key"))?;
        let credentials = VaultCredentials {
            address: client.address().to_owned(),
            unseal_key,
            root_token: init.root_token,
        };
        save_credentials(&dir, node_name, &credentials)?;
        let _ = progress.send_status(format!("Vault {node_name} - initialised"), StatusKind::Done);
        credentials
    };

    // A newly initialised Vault is sealed.
    if !status.initialized || status.sealed {
        let status = client
            .unseal(&credentials.unseal_key)
            .await
            .with_context(|| format!("Failed to unseal Vault node '{node_name}'"))?;
        if status.sealed {
            bail!("Vault node '{node_name}' is still sealed after its unseal key was applied");
        }
        let _ = progress.send_status(format!("Vault {node_name} - unsealed"), StatusKind::Done);
    }
    Ok(credentials.into_info(node_name))
}

/// Poll the seal status until the API answers, within `timeout`.
async fn wait_for_api(
    client: &VaultClient,
    node_name: &str,
    timeout: Duration,
) -> Result<SealStatus> {
    let started = Instant::now();
    loop {
        match client.seal_status().await {
            Ok(status) => return Ok(status),
            Err(error) if started.elapsed() < timeout => {
                tracing::debug!(%node_name, error = %format!("{error:#}"), "Vault API not ready yet");
                tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
            }
            Err(error) => {
                return Err(error.context(format!(
                    "Vault node '{node_name}' API did not answer within {}s",
                    timeout.as_secs()
                )));
            }
        }
    }
}

/// The credentials of every Vault node in a lab's Vault directory (see [`vault_dir`]), for
/// `sherpa download`.
#[instrument(level = "debug", fields(dir = %dir.display()))]
pub fn read_vault_credentials(dir: &Path) -> Result<Vec<VaultInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut vaults = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to list {}", dir.display()))? {
        let path = entry?.path();
        if path.extension() != Some(OsStr::new("toml")) {
            continue;
        }
        let Some(node_name) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(credentials) = load_credentials(dir, node_name)? {
            vaults.push(credentials.into_info(node_name));
        }
    }
    vaults.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(vaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    fn credentials(token: &str) -> VaultCredentials {
        VaultCredentials {
            address: "https://172.31.1.21:8200".to_owned(),
            unseal_key: "oQ==".to_owned(),
            root_token: token.to_owned(),
        }
    }

    #[test]
    fn only_vault_nodes_with_the_generated_config_are_initialised() {
        assert!(sherpa_initialises_vault(&NodeModel::HashicorpVault, false));
        assert!(!sherpa_initialises_vault(&NodeModel::HashicorpVault, true));
        assert!(!sherpa_initialises_vault(&NodeModel::AristaCeos, false));
    }

    #[test]
    fn credentials_are_saved_owner_only_and_read_back() {
        let dir = TempDir::new().unwrap();
        save_credentials(dir.path(), "vault01", &credentials("hvs.one")).unwrap();
        let path = credentials_path(dir.path(), "vault01");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(credentials("hvs.one"))
        );
        assert_eq!(load_credentials(dir.path(), "vault02").unwrap(), None);
    }

    #[test]
    fn download_lists_each_vault_by_name() {
        let dir = TempDir::new().unwrap();
        save_credentials(dir.path(), "vault02", &credentials("hvs.two")).unwrap();
        save_credentials(dir.path(), "vault01", &credentials("hvs.one")).unwrap();
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let vaults = read_vault_credentials(dir.path()).unwrap();
        let names: Vec<&str> = vaults.iter().map(|vault| vault.name.as_str()).collect();
        assert_eq!(names, ["vault01", "vault02"]);
        assert_eq!(vaults[1].root_token, "hvs.two");
        assert!(
            read_vault_credentials(&dir.path().join("missing"))
                .unwrap()
                .is_empty()
        );
    }
}
