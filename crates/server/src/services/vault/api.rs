//! The Vault API calls Sherpa makes: seal status, initialise and unseal, and, with the root
//! token, listing and enabling mounts and writing configuration. Every call goes over TLS to
//! the node's management address and trusts only the lab CA.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use shared::konst::VAULT_API_PORT;

/// Timeout of one API request, other than init.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout of the init request. With Raft storage, init bootstraps a one-node cluster, waits for
/// it to elect itself leader and writes the keyring, root token and default mounts, each write
/// synced to disk: about 6.5s on an idle server, and much longer with the CPU and disk busy
/// starting a lab. 90s is Vault's own default `max_request_duration`, the longest Vault lets a
/// request run.
const INIT_TIMEOUT: Duration = Duration::from_secs(90);
/// Key shares Sherpa initialises Vault with: one, which unseals Vault on its own.
const VAULT_KEY_SHARES: u8 = 1;
/// Header carrying the Vault token of an authenticated request.
const VAULT_TOKEN_HEADER: &str = "X-Vault-Token";

/// `GET /v1/sys/seal-status`, the fields Sherpa reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct SealStatus {
    pub initialized: bool,
    pub sealed: bool,
}

/// `PUT /v1/sys/init` body.
#[derive(Debug, Serialize)]
struct InitRequest {
    secret_shares: u8,
    secret_threshold: u8,
}

/// `PUT /v1/sys/init` response, the fields Sherpa keeps.
#[derive(Debug, Deserialize)]
pub struct InitResponse {
    pub keys_base64: Vec<String>,
    pub root_token: String,
}

/// The kinds of Vault mount Sherpa lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    /// Secrets engines, `sys/mounts`.
    SecretsEngine,
    /// Auth methods, `sys/auth`.
    AuthMethod,
}

impl MountKind {
    fn list_path(self) -> &'static str {
        match self {
            MountKind::SecretsEngine => "sys/mounts",
            MountKind::AuthMethod => "sys/auth",
        }
    }
}

/// `GET /v1/sys/mounts` and `GET /v1/sys/auth` responses: mounts keyed by path.
#[derive(Debug, Deserialize)]
struct MountList {
    data: HashMap<String, Value>,
}

/// `PUT /v1/sys/unseal` body.
#[derive(Debug, Serialize)]
struct UnsealRequest<'a> {
    key: &'a str,
}

/// The API address of a Vault node.
pub fn vault_address(node_address: Ipv4Addr) -> String {
    format!("https://{node_address}:{VAULT_API_PORT}")
}

/// A Vault node's API, reached over TLS trusting only the lab CA.
#[derive(Debug)]
pub struct VaultClient {
    client: reqwest::Client,
    address: String,
    init_timeout: Duration,
}

impl VaultClient {
    pub fn new(node_address: Ipv4Addr, lab_ca_pem: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(lab_ca_pem.as_bytes())
                .context("Failed to load the lab CA certificate")?])
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("Failed to build the Vault API client")?;
        Ok(Self {
            client,
            address: vault_address(node_address),
            init_timeout: INIT_TIMEOUT,
        })
    }

    /// The same client with a shorter init timeout, so a test can make an init time out.
    #[cfg(test)]
    pub fn with_init_timeout(self, init_timeout: Duration) -> Self {
        Self {
            init_timeout,
            ..self
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub async fn seal_status(&self) -> Result<SealStatus> {
        let body = self
            .send(
                self.client
                    .get(format!("{}/v1/sys/seal-status", self.address)),
            )
            .await?;
        serde_json::from_slice(&body).context("Failed to parse the Vault seal status")
    }

    /// Initialise Vault with one key share. Vault answers with the unseal key and root token
    /// once only: an init whose response is lost may still have completed inside Vault.
    pub async fn init(&self) -> Result<InitResponse> {
        let request = InitRequest {
            secret_shares: VAULT_KEY_SHARES,
            secret_threshold: VAULT_KEY_SHARES,
        };
        let body = self
            .send(
                self.client
                    .put(format!("{}/v1/sys/init", self.address))
                    .timeout(self.init_timeout)
                    .body(serde_json::to_vec(&request)?),
            )
            .await?;
        serde_json::from_slice(&body).context("Failed to parse the Vault init response")
    }

    /// Paths of the enabled mounts of one kind, each with a trailing `/`.
    pub async fn mount_paths(&self, token: &str, kind: MountKind) -> Result<Vec<String>> {
        let list_path = kind.list_path();
        let body = self
            .send(
                self.client
                    .get(format!("{}/v1/{list_path}", self.address))
                    .header(VAULT_TOKEN_HEADER, token),
            )
            .await?;
        let mounts: MountList = serde_json::from_slice(&body)
            .with_context(|| format!("Failed to parse the Vault {list_path} list"))?;
        Ok(mounts.data.into_keys().collect())
    }

    /// Write `body` to the API path `path` (without `/v1/`), as the root token.
    pub async fn write(&self, token: &str, path: &str, body: &Value) -> Result<()> {
        self.send(
            self.client
                .post(format!("{}/v1/{path}", self.address))
                .header(VAULT_TOKEN_HEADER, token)
                .body(serde_json::to_vec(body)?),
        )
        .await
        .with_context(|| format!("Failed to write Vault path '{path}'"))?;
        Ok(())
    }

    /// Whether this node is the active node, the one that serves requests:
    /// `GET /v1/sys/health` answers 200 only then. An unsealed node is in standby until its
    /// storage has elected a leader.
    pub async fn is_active(&self) -> Result<bool> {
        let response = self
            .client
            .get(format!("{}/v1/sys/health", self.address))
            .send()
            .await
            .with_context(|| format!("Vault API at {} did not answer", self.address))?;
        Ok(response.status() == reqwest::StatusCode::OK)
    }

    pub async fn unseal(&self, key: &str) -> Result<SealStatus> {
        let body = self
            .send(
                self.client
                    .put(format!("{}/v1/sys/unseal", self.address))
                    .body(serde_json::to_vec(&UnsealRequest { key })?),
            )
            .await?;
        serde_json::from_slice(&body).context("Failed to parse the Vault unseal response")
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Vec<u8>> {
        let response = request
            .header("Content-Type", "application/json")
            .send()
            .await
            .with_context(|| format!("Vault API at {} did not answer", self.address))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .context("Failed to read the Vault API response")?;
        if !status.is_success() {
            bail!(
                "Vault API at {} answered {status}: {}",
                self.address,
                String::from_utf8_lossy(&body)
            );
        }
        Ok(body.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_use_one_key_share() {
        let body = serde_json::to_value(InitRequest {
            secret_shares: VAULT_KEY_SHARES,
            secret_threshold: VAULT_KEY_SHARES,
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "secret_shares": 1, "secret_threshold": 1 })
        );
        let unseal = serde_json::to_value(UnsealRequest { key: "abc=" }).unwrap();
        assert_eq!(unseal, serde_json::json!({ "key": "abc=" }));
    }

    #[test]
    fn responses_parse_the_fields_sherpa_reads() {
        let status: SealStatus = serde_json::from_str(
            r#"{"type":"shamir","initialized":true,"sealed":false,"t":1,"n":1,"progress":0}"#,
        )
        .unwrap();
        assert_eq!(
            status,
            SealStatus {
                initialized: true,
                sealed: false
            }
        );
        let init: InitResponse = serde_json::from_str(
            r#"{"keys":["a1"],"keys_base64":["oQ=="],"root_token":"hvs.root"}"#,
        )
        .unwrap();
        assert_eq!(init.keys_base64, ["oQ=="]);
        assert_eq!(init.root_token, "hvs.root");
    }

    #[test]
    fn mount_lists_are_keyed_by_path() {
        let mounts: MountList = serde_json::from_str(
            r#"{"request_id":"r","data":{"secret/":{"type":"kv"},"sys/":{"type":"system"}},"secret/":{"type":"kv"}}"#,
        )
        .unwrap();
        let mut paths: Vec<String> = mounts.data.into_keys().collect();
        paths.sort();
        assert_eq!(paths, ["secret/", "sys/"]);
    }

    #[test]
    fn the_api_is_on_the_management_address_over_https() {
        assert_eq!(
            vault_address(Ipv4Addr::new(172, 31, 1, 21)),
            "https://172.31.1.21:8200"
        );
    }
}
