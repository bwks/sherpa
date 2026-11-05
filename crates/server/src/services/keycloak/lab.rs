//! Keycloak nodes in the lab lifecycle. Sherpa makes the `sherpa` realm on each Keycloak node,
//! with a `kubernetes` client, groups bound to cluster roles, and two users: `sherpa`, with
//! the lab password, and the lab owner, with a generated password. The users and passwords are
//! kept in the lab directory, readable only by the server.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;

use shared::data::{KeycloakInfo, KeycloakUser, StatusKind};
use shared::konst::{
    KEYCLOAK_CLUSTER_ADMINS_GROUP, KEYCLOAK_CLUSTER_VIEWERS_GROUP, KEYCLOAK_KUBERNETES_CLIENT_ID,
    KEYCLOAK_REALM, READINESS_SLEEP, SHERPA_LABS_PATH, SHERPA_PASSWORD, SHERPA_USERNAME,
};

use super::api::KeycloakClient;
use crate::services::guest_account::random_secret;
use crate::services::progress::ProgressSender;
use crate::tls::generator::write_file_with_permissions;

/// Directory in the lab directory holding each Keycloak node's realm users.
const KEYCLOAK_DIR: &str = "keycloak";
/// Redirect addresses of the `kubectl oidc-login` plugin's browser login.
const KUBELOGIN_REDIRECT_URIS: [&str; 2] = ["http://localhost:8000", "http://localhost:18000"];

/// A Keycloak node's realm users, saved as `<lab>/keycloak/<node>.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RealmCredentials {
    address: String,
    users: Vec<KeycloakUser>,
}

impl RealmCredentials {
    fn into_info(self, name: &str) -> KeycloakInfo {
        KeycloakInfo {
            name: name.to_owned(),
            address: self.address,
            realm: KEYCLOAK_REALM.to_owned(),
            users: self.users,
        }
    }
}

/// The lab directory where Sherpa keeps its Keycloak nodes' realm users.
pub fn keycloak_dir(lab_id: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH).join(lab_id).join(KEYCLOAK_DIR)
}

fn credentials_path(dir: &Path, node_name: &str) -> PathBuf {
    dir.join(format!("{node_name}.toml"))
}

fn load_credentials(dir: &Path, node_name: &str) -> Result<Option<RealmCredentials>> {
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

/// The realm's users: those saved for the node, or `sherpa` and the lab owner with a new
/// password, saved before Keycloak hears of them.
fn realm_credentials(
    dir: &Path,
    node_name: &str,
    address: &str,
    owner: &str,
) -> Result<RealmCredentials> {
    if let Some(credentials) = load_credentials(dir, node_name)? {
        return Ok(credentials);
    }
    let mut users = vec![KeycloakUser {
        username: SHERPA_USERNAME.to_owned(),
        password: SHERPA_PASSWORD.to_owned(),
    }];
    if owner != SHERPA_USERNAME {
        users.push(KeycloakUser {
            username: owner.to_owned(),
            password: random_secret(),
        });
    }
    let credentials = RealmCredentials {
        address: address.to_owned(),
        users,
    };
    let content = toml::to_string(&credentials).context("Failed to serialize Keycloak users")?;
    write_file_with_permissions(&credentials_path(dir, node_name), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save the users of Keycloak node '{node_name}'"))?;
    Ok(credentials)
}

/// The `kubernetes` client: public, for `kubectl oidc-login`, with the realm's groups and the
/// client itself as the audience in its tokens.
fn kubernetes_client() -> Value {
    json!({
        "clientId": KEYCLOAK_KUBERNETES_CLIENT_ID,
        "publicClient": true,
        "standardFlowEnabled": true,
        "directAccessGrantsEnabled": true,
        "redirectUris": KUBELOGIN_REDIRECT_URIS,
        "attributes": { "pkce.code.challenge.method": "S256" },
        "protocolMappers": [
            {
                "name": "groups",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-group-membership-mapper",
                "config": {
                    "claim.name": "groups",
                    "full.path": "false",
                    "id.token.claim": "true",
                    "access.token.claim": "true",
                    "userinfo.token.claim": "true",
                },
            },
            {
                "name": "audience",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-audience-mapper",
                "config": {
                    "included.client.audience": KEYCLOAK_KUBERNETES_CLIENT_ID,
                    "id.token.claim": "true",
                    "access.token.claim": "true",
                },
            },
        ],
    })
}

/// A realm user in the cluster admins group, with a permanent password.
fn realm_user(user: &KeycloakUser) -> Value {
    json!({
        "username": user.username,
        "enabled": true,
        "emailVerified": true,
        "email": format!("{}@{KEYCLOAK_REALM}.lab.local", user.username),
        "firstName": user.username,
        "lastName": KEYCLOAK_REALM,
        "groups": [KEYCLOAK_CLUSTER_ADMINS_GROUP],
        "credentials": [{ "type": "password", "value": user.password, "temporary": false }],
    })
}

/// Wait for a Keycloak node, then make the `sherpa` realm with its client, groups and users.
/// Anything already there is left as it is.
#[instrument(skip(lab_ca_pem, progress), fields(%lab_id, %node_name, %node_address))]
pub async fn ensure_keycloak_realm(
    lab_id: &str,
    node_name: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    owner: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<KeycloakInfo> {
    let client = KeycloakClient::new(node_address, lab_ca_pem)?;
    wait_until_ready(&client, node_name, timeout).await?;
    let credentials = realm_credentials(&keycloak_dir(lab_id), node_name, client.address(), owner)?;

    let token = client
        .admin_token(SHERPA_USERNAME, SHERPA_PASSWORD)
        .await
        .with_context(|| format!("Failed to log in to Keycloak node '{node_name}'"))?;
    client
        .create(
            &token,
            "",
            &json!({ "realm": KEYCLOAK_REALM, "enabled": true }),
        )
        .await?;
    let realm = format!("/{KEYCLOAK_REALM}");
    client
        .create(&token, &format!("{realm}/clients"), &kubernetes_client())
        .await?;
    for group in [
        KEYCLOAK_CLUSTER_ADMINS_GROUP,
        KEYCLOAK_CLUSTER_VIEWERS_GROUP,
    ] {
        client
            .create(
                &token,
                &format!("{realm}/groups"),
                &json!({ "name": group }),
            )
            .await?;
    }
    for user in &credentials.users {
        client
            .create(&token, &format!("{realm}/users"), &realm_user(user))
            .await?;
    }
    let _ = progress.send_status(
        format!("Keycloak {node_name} - realm {KEYCLOAK_REALM} ready"),
        StatusKind::Done,
    );
    Ok(credentials.into_info(node_name))
}

/// Poll Keycloak until it serves its OpenID configuration, within `timeout`.
async fn wait_until_ready(
    client: &KeycloakClient,
    node_name: &str,
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    loop {
        match client.ready().await {
            Ok(()) => return Ok(()),
            Err(error) if started.elapsed() < timeout => {
                tracing::debug!(%node_name, error = %format!("{error:#}"), "Keycloak not ready yet");
                tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
            }
            Err(error) => {
                return Err(error.context(format!(
                    "Keycloak node '{node_name}' was not ready within {}s",
                    timeout.as_secs()
                )));
            }
        }
    }
}

/// The realm users of every Keycloak node in a lab's Keycloak directory (see
/// [`keycloak_dir`]), for `sherpa download`.
#[instrument(level = "debug", fields(dir = %dir.display()))]
pub fn read_keycloak_credentials(dir: &Path) -> Result<Vec<KeycloakInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut keycloaks = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to list {}", dir.display()))? {
        let path = entry?.path();
        if path.extension() != Some(OsStr::new("toml")) {
            continue;
        }
        let Some(node_name) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(credentials) = load_credentials(dir, node_name)? {
            keycloaks.push(credentials.into_info(node_name));
        }
    }
    keycloaks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(keycloaks)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    #[test]
    fn the_realm_has_sherpa_and_the_owner_with_a_saved_password() {
        let dir = TempDir::new().unwrap();
        let created =
            realm_credentials(dir.path(), "kc01", "https://172.31.1.23:8443", "alice").unwrap();
        let usernames: Vec<&str> = created
            .users
            .iter()
            .map(|user| user.username.as_str())
            .collect();
        assert_eq!(usernames, ["sherpa", "alice"]);
        assert_eq!(created.users[0].password, SHERPA_PASSWORD);
        assert_ne!(created.users[1].password, SHERPA_PASSWORD);

        let mode = fs::metadata(credentials_path(dir.path(), "kc01"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let again =
            realm_credentials(dir.path(), "kc01", "https://172.31.1.23:8443", "alice").unwrap();
        assert_eq!(again, created);
    }

    #[test]
    fn an_owner_named_sherpa_is_one_user() {
        let dir = TempDir::new().unwrap();
        let created =
            realm_credentials(dir.path(), "kc01", "https://172.31.1.23:8443", "sherpa").unwrap();
        assert_eq!(created.users.len(), 1);
    }

    #[test]
    fn realm_users_are_cluster_admins_with_permanent_passwords() {
        let user = realm_user(&KeycloakUser {
            username: "alice".to_owned(),
            password: "pw".to_owned(),
        });
        assert_eq!(user["groups"], json!(["cluster-admins"]));
        assert_eq!(user["credentials"][0]["value"], "pw");
        assert_eq!(user["credentials"][0]["temporary"], false);
    }

    #[test]
    fn the_kubernetes_client_puts_groups_and_its_audience_in_tokens() {
        let client = kubernetes_client();
        assert_eq!(client["clientId"], "kubernetes");
        assert_eq!(client["publicClient"], true);
        let mappers: Vec<&str> = client["protocolMappers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mapper| mapper["protocolMapper"].as_str().unwrap())
            .collect();
        assert_eq!(
            mappers,
            ["oidc-group-membership-mapper", "oidc-audience-mapper"]
        );
    }

    #[test]
    fn download_lists_each_keycloak_by_name() {
        let dir = TempDir::new().unwrap();
        realm_credentials(dir.path(), "kc02", "https://172.31.1.24:8443", "alice").unwrap();
        realm_credentials(dir.path(), "kc01", "https://172.31.1.23:8443", "alice").unwrap();
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let keycloaks = read_keycloak_credentials(dir.path()).unwrap();
        let names: Vec<&str> = keycloaks.iter().map(|kc| kc.name.as_str()).collect();
        assert_eq!(names, ["kc01", "kc02"]);
        assert_eq!(keycloaks[0].realm, "sherpa");
        assert_eq!(keycloaks[1].address, "https://172.31.1.24:8443");
        assert!(
            read_keycloak_credentials(&dir.path().join("missing"))
                .unwrap()
                .is_empty()
        );
    }
}
