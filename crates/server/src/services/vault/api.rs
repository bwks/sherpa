//! The Vault API calls Sherpa makes: seal status, initialise and unseal. Every call goes over
//! TLS to the node's management address and trusts only the lab CA.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use shared::konst::VAULT_API_PORT;

/// Timeout of one API request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Key shares Sherpa initialises Vault with: one, which unseals Vault on its own.
const VAULT_KEY_SHARES: u8 = 1;

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
        })
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

    /// Initialise Vault with one key share.
    pub async fn init(&self) -> Result<InitResponse> {
        let request = InitRequest {
            secret_shares: VAULT_KEY_SHARES,
            secret_threshold: VAULT_KEY_SHARES,
        };
        let body = self
            .send(
                self.client
                    .put(format!("{}/v1/sys/init", self.address))
                    .body(serde_json::to_vec(&request)?),
            )
            .await?;
        serde_json::from_slice(&body).context("Failed to parse the Vault init response")
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
    fn the_api_is_on_the_management_address_over_https() {
        assert_eq!(
            vault_address(Ipv4Addr::new(172, 31, 1, 21)),
            "https://172.31.1.21:8200"
        );
    }
}
