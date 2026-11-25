use std::path::Path;

use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::VaultInfo;
use shared::util::{Emoji, LabFolder};

use crate::lab_files::LabFile;

/// A Vault node's credentials file, `vault-<node>.toml`.
#[derive(Debug, Serialize)]
struct VaultCredentialsFile<'a> {
    address: &'a str,
    root_token: &'a str,
    unseal_key: &'a str,
}

/// The credentials file of one Vault node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VaultFile {
    pub name: String,
    pub address: String,
    pub file: LabFile,
}

/// Each Vault node's address, root token and unseal key as its file in `folder`,
/// `.sherpa/vault/vault-<node>.toml`, a secret. Writes nothing.
pub(crate) fn vault_credential_files(
    folder: &LabFolder,
    vaults: &[VaultInfo],
) -> Result<Vec<VaultFile>> {
    let mut files = Vec::with_capacity(vaults.len());
    for vault in vaults {
        let content = toml::to_string(&VaultCredentialsFile {
            address: &vault.address,
            root_token: &vault.root_token,
            unseal_key: &vault.unseal_key,
        })
        .context("Failed to serialize Vault credentials")?;
        files.push(VaultFile {
            name: vault.name.clone(),
            address: vault.address.clone(),
            file: LabFile {
                path: folder.vault(&vault.name),
                content,
                secret: Some("Vault credentials"),
            },
        });
    }
    Ok(files)
}

/// Print where each Vault node's credentials were written, and how to point the `vault` CLI at
/// it with the lab CA.
pub(crate) fn print_vault_files(files: &[VaultFile], lab_ca: Option<&Path>) {
    for vault in files {
        println!(
            "{} Vault {} credentials: {}",
            Emoji::Success,
            vault.name,
            vault.file.path.display()
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
    use std::fs;

    use super::*;

    use crate::lab_files::write_lab_file;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn lab_credentials_006_a_vault_file_has_its_token_and_key_and_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let vaults = [VaultInfo {
            name: "vault01".to_owned(),
            address: "https://172.31.1.21:8200".to_owned(),
            root_token: "hvs.root".to_owned(),
            unseal_key: "oQ==".to_owned(),
        }];
        let folder = LabFolder::new(dir.path());
        let files = vault_credential_files(&folder, &vaults).unwrap();
        let [vault] = files.as_slice() else {
            panic!("one file for one Vault node: {files:?}");
        };
        write_lab_file(&folder, &vault.file).unwrap();
        let path = dir.path().join(".sherpa/vault/vault-vault01.toml");
        assert_eq!(
            (
                vault.name.as_str(),
                vault.address.as_str(),
                &vault.file.path
            ),
            ("vault01", "https://172.31.1.21:8200", &path)
        );
        let content: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["root_token"].as_str(), Some("hvs.root"));
        assert_eq!(content["unseal_key"].as_str(), Some("oQ=="));
        assert_eq!(
            content["address"].as_str(),
            Some("https://172.31.1.21:8200")
        );
        #[cfg(unix)]
        {
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
