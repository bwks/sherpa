use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::{KeycloakInfo, KeycloakUser};
use shared::util::Emoji;

use crate::private_key::write_secret_file;

/// A Keycloak node's realm users file, `keycloak-<node>.toml`.
#[derive(Debug, Serialize)]
struct KeycloakCredentialsFile<'a> {
    address: &'a str,
    realm: &'a str,
    /// The realm's issuer, which clusters accept tokens from.
    issuer: String,
    /// Where the realm's users manage their account.
    account_console: String,
    users: &'a [KeycloakUser],
}

/// The users file written for one Keycloak node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeycloakFile {
    pub name: String,
    pub issuer: String,
    pub path: PathBuf,
}

/// Write each Keycloak node's realm, issuer and users into `dir` as `keycloak-<node>.toml`,
/// readable only by the current user.
#[tracing::instrument(level = "debug", skip(keycloaks), fields(dir = %dir.display(), keycloaks = keycloaks.len()))]
pub(crate) fn write_keycloak_credentials(
    dir: &Path,
    keycloaks: &[KeycloakInfo],
) -> Result<Vec<KeycloakFile>> {
    let mut written = Vec::with_capacity(keycloaks.len());
    for keycloak in keycloaks {
        let path = dir.join(format!("keycloak-{}.toml", keycloak.name));
        let issuer = format!("{}/realms/{}", keycloak.address, keycloak.realm);
        let content = toml::to_string(&KeycloakCredentialsFile {
            address: &keycloak.address,
            realm: &keycloak.realm,
            account_console: format!("{issuer}/account"),
            issuer: issuer.clone(),
            users: &keycloak.users,
        })
        .context("Failed to serialize Keycloak users")?;
        write_secret_file(&path, &content, "Keycloak users")?;
        written.push(KeycloakFile {
            name: keycloak.name.clone(),
            issuer,
            path,
        });
    }
    Ok(written)
}

/// Print where each Keycloak node's realm users were written, and the realm's issuer.
pub(crate) fn print_keycloak_files(files: &[KeycloakFile]) {
    for keycloak in files {
        println!(
            "{} Keycloak {} users: {}",
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
    fn each_keycloak_gets_an_owner_only_users_file() {
        let dir = tempfile::tempdir().unwrap();
        let keycloaks = [KeycloakInfo {
            name: "kc01".to_owned(),
            address: "https://172.31.1.23:8443".to_owned(),
            realm: "sherpa".to_owned(),
            users: vec![KeycloakUser {
                username: "alice".to_owned(),
                password: "s3cret".to_owned(),
            }],
        }];
        let written = write_keycloak_credentials(dir.path(), &keycloaks).unwrap();
        let path = dir.path().join("keycloak-kc01.toml");
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
            content["issuer"].as_str(),
            Some("https://172.31.1.23:8443/realms/sherpa")
        );
        assert_eq!(
            content["account_console"].as_str(),
            Some("https://172.31.1.23:8443/realms/sherpa/account")
        );
        assert_eq!(content["users"][0]["username"].as_str(), Some("alice"));
        assert_eq!(content["users"][0]["password"].as_str(), Some("s3cret"));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
