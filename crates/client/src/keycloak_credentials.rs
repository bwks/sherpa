use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::{AdminCredential, KeycloakInfo, KeycloakUser};
use shared::util::{Emoji, LabFolder};

use crate::lab_files::LabFile;

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
    /// The node's own admin, in the `master` realm. A node with none saved gets one when it is
    /// redeployed.
    #[serde(skip_serializing_if = "Option::is_none")]
    admin: Option<&'a AdminCredential>,
    users: &'a [KeycloakUser],
}

/// The users file of one Keycloak node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeycloakFile {
    pub name: String,
    pub issuer: String,
    pub file: LabFile,
}

/// Each Keycloak node's admin, realm, issuer and users as its file in `folder`,
/// `.sherpa/keycloak/keycloak-<node>.toml`, a secret. Writes nothing.
pub(crate) fn keycloak_credential_files(
    folder: &LabFolder,
    keycloaks: &[KeycloakInfo],
) -> Result<Vec<KeycloakFile>> {
    let mut files = Vec::with_capacity(keycloaks.len());
    for keycloak in keycloaks {
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
        files.push(KeycloakFile {
            name: keycloak.name.clone(),
            issuer,
            file: LabFile {
                path: folder.keycloak(&keycloak.name),
                content,
                secret: Some("Keycloak credentials"),
            },
        });
    }
    Ok(files)
}

/// Print where each Keycloak node's admin and realm users were written, and the realm's issuer.
pub(crate) fn print_keycloak_files(files: &[KeycloakFile]) {
    for keycloak in files {
        println!(
            "{} Keycloak {} admin and users: {}",
            Emoji::Success,
            keycloak.name,
            keycloak.file.path.display()
        );
        println!("   Realm issuer: {}", keycloak.issuer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lab_files::write_lab_file;

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
        let folder = LabFolder::new(dir.path());
        let files = keycloak_credential_files(&folder, &keycloaks).unwrap();
        let [keycloak] = files.as_slice() else {
            panic!("one file for one Keycloak node: {files:?}");
        };
        write_lab_file(&folder, &keycloak.file).unwrap();
        let path = dir.path().join(".sherpa/keycloak/keycloak-kc01.toml");
        assert_eq!(
            (
                keycloak.name.as_str(),
                keycloak.issuer.as_str(),
                &keycloak.file.path
            ),
            ("kc01", "https://172.31.1.23:8443/realms/sherpa", &path)
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
