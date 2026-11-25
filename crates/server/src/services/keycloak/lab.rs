//! Keycloak nodes in the lab lifecycle. Each Keycloak node starts with its own `master` realm
//! admin, `admin` with a generated password, and Sherpa makes the `sherpa` realm on it, with a
//! `kubernetes` client, groups bound to cluster roles, and the lab owner as its user, with a
//! generated password. The admin and the users are generated when the node is created, kept
//! across `redeploy`, and saved in the lab directory, readable only by the server.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;
use tracing::instrument;

use shared::data::{AdminCredential, KeycloakInfo, KeycloakUser, StatusKind};
use shared::konst::{
    KEYCLOAK_CLUSTER_ADMINS_GROUP, KEYCLOAK_CLUSTER_VIEWERS_GROUP, KEYCLOAK_KUBERNETES_CLIENT_ID,
    SHERPA_LABS_PATH,
};
use shared::util::write_file_with_permissions;

use super::api::KeycloakClient;
use crate::konst::{KEYCLOAK_REALM, READINESS_SLEEP};
use crate::services::admin_credentials::random_password;
use crate::services::progress::ProgressSender;

/// Directory in the lab directory holding each Keycloak node's admin and realm users.
pub(crate) const KEYCLOAK_DIR: &str = "keycloak";
/// Redirect addresses of the `kubectl oidc-login` plugin's browser login.
const KUBELOGIN_REDIRECT_URIS: [&str; 2] = ["http://localhost:8000", "http://localhost:18000"];
/// The `master` realm admin Keycloak creates at its first start, Keycloak's own default name.
const KEYCLOAK_ADMIN_USERNAME: &str = "admin";
/// The realm user shared by Keycloak nodes with no admin of their own, with the password Sherpa
/// published. A node's saved users lose it when the node gets its own admin. No Sherpa user can
/// have the name, so it is never the lab owner.
const LEGACY_SHARED_REALM_USER: &str = "sherpa";

/// A Keycloak node's admin and realm users, saved as `<lab>/keycloak/<node>.toml`. A file with
/// no admin leaves its node on the shared admin until it is redeployed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RealmCredentials {
    address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    admin: Option<AdminCredential>,
    users: Vec<KeycloakUser>,
}

impl RealmCredentials {
    fn into_info(self, name: &str) -> KeycloakInfo {
        KeycloakInfo {
            name: name.to_owned(),
            address: self.address,
            realm: KEYCLOAK_REALM.to_owned(),
            admin: self.admin,
            users: self.users,
        }
    }
}

/// The lab directory where Sherpa keeps its Keycloak nodes' admins and realm users.
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

/// The admin a Keycloak node is created with, saved in `dir` with the realm's users before the
/// node starts. A node keeps what was saved for it, across `redeploy`: Keycloak's database is
/// in the container, so a redeployed node starts afresh with the same admin and users, and the
/// owner's downloaded file stays right. A new node gets an admin and the lab owner as its realm
/// user, each with a new password. A node saved with no admin of its own gets
/// one, and loses the realm user with the published password.
#[instrument(skip_all, fields(dir = %dir.display(), %node_name), level = "debug")]
pub(crate) fn ensure_keycloak_credentials(
    dir: &Path,
    node_name: &str,
    address: &str,
    owner: &str,
) -> Result<AdminCredential> {
    let credentials = match load_credentials(dir, node_name)? {
        Some(RealmCredentials {
            admin: Some(admin), ..
        }) => return Ok(admin),
        Some(mut saved) => {
            saved
                .users
                .retain(|user| user.username != LEGACY_SHARED_REALM_USER);
            RealmCredentials {
                admin: Some(new_admin()),
                ..saved
            }
        }
        None => RealmCredentials {
            address: address.to_owned(),
            admin: Some(new_admin()),
            users: vec![KeycloakUser {
                username: owner.to_owned(),
                password: random_password(),
            }],
        },
    };
    let content = toml::to_string(&credentials).context("Failed to serialize Keycloak users")?;
    write_file_with_permissions(&credentials_path(dir, node_name), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save the users of Keycloak node '{node_name}'"))?;
    credentials
        .admin
        .with_context(|| format!("Keycloak node '{node_name}' has no admin"))
}

/// A new `master` realm admin with a random password.
fn new_admin() -> AdminCredential {
    AdminCredential {
        username: KEYCLOAK_ADMIN_USERNAME.to_owned(),
        password: random_password(),
    }
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

/// Wait for a Keycloak node, then log in as its admin and make the `sherpa` realm with its
/// client, groups and users, all as saved when the node was created
/// ([`ensure_keycloak_credentials`]). Anything already there is left as it is.
#[instrument(skip(lab_ca_pem, progress), fields(%lab_id, %node_name, %node_address))]
pub async fn ensure_keycloak_realm(
    lab_id: &str,
    node_name: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<KeycloakInfo> {
    let client = KeycloakClient::new(node_address, lab_ca_pem)?;
    wait_until_ready(&client, node_name, timeout).await?;
    let credentials = load_credentials(&keycloak_dir(lab_id), node_name)?.with_context(|| {
        format!("Keycloak node '{node_name}' has no saved admin; redeploy it to give it one")
    })?;
    let admin = credentials.admin.as_ref().with_context(|| {
        format!("Keycloak node '{node_name}' has no saved admin; redeploy it to give it one")
    })?;

    let token = client
        .admin_token(&admin.username, &admin.password)
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

/// Poll Keycloak until it serves its `OpenID` configuration, within `timeout`.
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
                time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
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

    const ADDRESS: &str = "https://172.31.1.23:8443";

    #[test]
    fn lab_credentials_009_a_new_keycloak_node_gets_its_admin_and_the_owner_saved_for_sherpad() {
        let dir = TempDir::new().unwrap();
        let admin = ensure_keycloak_credentials(dir.path(), "kc01", ADDRESS, "alice").unwrap();
        assert_eq!(admin.username, "admin");
        assert_eq!(admin.password.len(), 20);

        let saved = load_credentials(dir.path(), "kc01").unwrap().unwrap();
        assert_eq!(saved.admin.as_ref(), Some(&admin));
        assert_eq!(saved.address, ADDRESS);
        let usernames: Vec<&str> = saved
            .users
            .iter()
            .map(|user| user.username.as_str())
            .collect();
        assert_eq!(usernames, ["alice"]);
        assert_ne!(saved.users[0].password, admin.password);

        let mode = fs::metadata(credentials_path(dir.path(), "kc01"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn upgraded_labs_001_a_keycloak_node_from_before_its_own_admin_gets_one_and_loses_the_shared_user()
     {
        let dir = TempDir::new().unwrap();
        // A 0.6 file: the shared realm user with the password every node had, and the owner.
        let published = "old-shared-password";
        let old = format!(
            r#"address = "{ADDRESS}"

[[users]]
username = "sherpa"
password = "{published}"

[[users]]
username = "alice"
password = "owner-pw"
"#
        );
        fs::write(credentials_path(dir.path(), "kc01"), old).unwrap();

        let admin = ensure_keycloak_credentials(dir.path(), "kc01", ADDRESS, "alice").unwrap();
        assert_eq!(admin.username, "admin");
        assert_ne!(admin.password, published);
        let saved = load_credentials(dir.path(), "kc01").unwrap().unwrap();
        assert_eq!(saved.admin, Some(admin));
        assert_eq!(
            saved.users,
            [KeycloakUser {
                username: "alice".to_owned(),
                password: "owner-pw".to_owned(),
            }]
        );
        let content = fs::read_to_string(credentials_path(dir.path(), "kc01")).unwrap();
        assert!(!content.contains(published), "{content}");
    }

    #[test]
    fn keycloak_realm_002_realm_users_are_cluster_admins_with_permanent_passwords() {
        let user = realm_user(&KeycloakUser {
            username: "alice".to_owned(),
            password: "pw".to_owned(),
        });
        assert_eq!(user["groups"], json!(["cluster-admins"]));
        assert_eq!(user["credentials"][0]["value"], "pw");
        assert_eq!(user["credentials"][0]["temporary"], false);
    }

    #[test]
    fn keycloak_realm_003_the_kubernetes_client_puts_groups_and_its_audience_in_tokens() {
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
}
