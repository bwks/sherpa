use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::VaultInfo;
use shared::util::Emoji;

use crate::private_key::write_secret_file;

/// A Vault node's credentials file, `vault-<node>.toml`.
#[derive(Debug, Serialize)]
struct VaultCredentialsFile<'a> {
    address: &'a str,
    root_token: &'a str,
    unseal_key: &'a str,
}

/// The credentials file written for one Vault node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VaultFile {
    pub name: String,
    pub address: String,
    pub path: PathBuf,
}

/// Write each Vault node's address, root token and unseal key into `dir` as
/// `vault-<node>.toml`, readable only by the current user.
#[tracing::instrument(level = "debug", skip(vaults), fields(dir = %dir.display(), vaults = vaults.len()))]
pub(crate) fn write_vault_credentials(dir: &Path, vaults: &[VaultInfo]) -> Result<Vec<VaultFile>> {
    let mut written = Vec::with_capacity(vaults.len());
    for vault in vaults {
        let path = dir.join(format!("vault-{}.toml", vault.name));
        let content = toml::to_string(&VaultCredentialsFile {
            address: &vault.address,
            root_token: &vault.root_token,
            unseal_key: &vault.unseal_key,
        })
        .context("Failed to serialize Vault credentials")?;
        write_secret_file(&path, &content, "Vault credentials")?;
        written.push(VaultFile {
            name: vault.name.clone(),
            address: vault.address.clone(),
            path,
        });
    }
    Ok(written)
}

/// Print where each Vault node's credentials were written, and how to point the `vault` CLI at
/// it with the lab CA.
pub(crate) fn print_vault_files(files: &[VaultFile], lab_ca: Option<&Path>) {
    for vault in files {
        println!(
            "{} Vault {} credentials: {}",
            Emoji::Success,
            vault.name,
            vault.path.display()
        );
        match lab_ca {
            Some(lab_ca) => println!(
                "   export VAULT_ADDR={} VAULT_CACERT={}",
                vault.address,
                lab_ca.display()
            ),
            None => println!("   export VAULT_ADDR={}", vault.address),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn each_vault_gets_an_owner_only_credentials_file() {
        let dir = tempfile::tempdir().unwrap();
        let vaults = [VaultInfo {
            name: "vault01".to_owned(),
            address: "https://172.31.1.21:8200".to_owned(),
            root_token: "hvs.root".to_owned(),
            unseal_key: "oQ==".to_owned(),
        }];
        let written = write_vault_credentials(dir.path(), &vaults).unwrap();
        let path = dir.path().join("vault-vault01.toml");
        assert_eq!(
            written,
            [VaultFile {
                name: "vault01".to_owned(),
                address: "https://172.31.1.21:8200".to_owned(),
                path: path.clone(),
            }]
        );
        let content: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["root_token"].as_str(), Some("hvs.root"));
        assert_eq!(content["unseal_key"].as_str(), Some("oQ=="));
        assert_eq!(
            content["address"].as_str(),
            Some("https://172.31.1.21:8200")
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
