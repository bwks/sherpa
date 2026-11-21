use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::{AdminCredential, KeycloakInfo, KeycloakUser};
use shared::util::{Emoji, LabFolder};

use crate::lab_files::ensure_lab_subdir;
use crate::secret_file::write_secret_file;

/// A Keycloak node's credentials file, `keycloak-<node>.toml`: its admin and its realm users.
#[derive(Debug, Serialize)]
struct KeycloakCredentialsFile<'a> {
    address: &'a str,
    realm: &'a str,
    /// The realm's issuer, which clusters accept tokens from.
    issuer: String,
    /// Where the realm's users manage their account.
    account_console: String,
    /// Where the admin logs in to the `master` realm.
    admin_console: String,
    /// The node's own admin, in the `master` realm. A node created before Sherpa 0.7.0 has none
    /// until it is redeployed.
    #[serde(skip_serializing_if = "Option::is_none")]
    admin: Option<&'a AdminCredential>,
    users: &'a [KeycloakUser],
}

/// The users file written for one Keycloak node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeycloakFile {
    pub name: String,
    pub issuer: String,
    pub path: PathBuf,
}

/// Write each Keycloak node's admin, realm, issuer and users into `folder` as
/// `.sherpa/keycloak/keycloak-<node>.toml`, readable only by the current user.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display(), keycloaks = keycloaks.len()))]
pub(crate) fn write_keycloak_credentials(
    folder: &LabFolder,
    keycloaks: &[KeycloakInfo],
) -> Result<Vec<KeycloakFile>> {
    if !keycloaks.is_empty() {
        ensure_lab_subdir(folder, &folder.keycloak_dir())?;
    }
    let mut written = Vec::with_capacity(keycloaks.len());
    for keycloak in keycloaks {
        let path = folder.keycloak(&keycloak.name);
        let issuer = format!("{}/realms/{}", keycloak.address, keycloak.realm);
        let content = toml::to_string(&KeycloakCredentialsFile {
            address: &keycloak.address,
            realm: &keycloak.realm,
            account_console: format!("{issuer}/account"),
            issuer: issuer.clone(),
            admin_console: format!("{}/admin/master/console/", keycloak.address),
            admin: keycloak.admin.as_ref(),
            users: &keycloak.users,
        })
        .context("Failed to serialize Keycloak credentials")?;
        write_secret_file(&path, &content, "Keycloak credentials")?;
        written.push(KeycloakFile {
            name: keycloak.name.clone(),
            issuer,
            path,
        });
    }
    Ok(written)
}

/// Print where each Keycloak node's admin and realm users were written, and the realm's issuer.
pub(crate) fn print_keycloak_files(files: &[KeycloakFile]) {
    for keycloak in files {
        println!(
            "{} Keycloak {} admin and users: {}",
            Emoji::Success,
            keycloak.name,
            keycloak.path.display()
        );
        println!("   Realm issuer: {}", keycloak.issuer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn lab_credentials_007_a_keycloak_file_has_its_addresses_and_users_and_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let keycloaks = [KeycloakInfo {
            name: "kc01".to_owned(),
            address: "https://172.31.1.23:8443".to_owned(),
            realm: "sherpa".to_owned(),
            admin: Some(AdminCredential {
                username: "admin".to_owned(),
                password: "kc-admin-pw".to_owned(),
            }),
            users: vec![KeycloakUser {
                username: "alice".to_owned(),
                password: "s3cret".to_owned(),
            }],
        }];
        let written = write_keycloak_credentials(&LabFolder::new(dir.path()), &keycloaks).unwrap();
        let path = dir.path().join(".sherpa/keycloak/keycloak-kc01.toml");
        assert_eq!(
            written,
            [KeycloakFile {
                name: "kc01".to_owned(),
                issuer: "https://172.31.1.23:8443/realms/sherpa".to_owned(),
                path: path.clone(),
            }]
        );
        let content: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            content["address"].as_str(),
            Some("https://172.31.1.23:8443")
        );
        assert_eq!(content["realm"].as_str(), Some("sherpa"));
        assert_eq!(
            content["issuer"].as_str(),
            Some("https://172.31.1.23:8443/realms/sherpa")
        );
        assert_eq!(
            content["account_console"].as_str(),
            Some("https://172.31.1.23:8443/realms/sherpa/account")
        );
        assert_eq!(
            content["admin_console"].as_str(),
            Some("https://172.31.1.23:8443/admin/master/console/")
        );
        assert_eq!(content["admin"]["username"].as_str(), Some("admin"));
        assert_eq!(content["admin"]["password"].as_str(), Some("kc-admin-pw"));
        assert_eq!(content["users"][0]["username"].as_str(), Some("alice"));
        assert_eq!(content["users"][0]["password"].as_str(), Some("s3cret"));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
