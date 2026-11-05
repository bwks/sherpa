//! The Keycloak API calls Sherpa makes: readiness, an admin token, and creating realm
//! resources. Every call goes over TLS to the node's management address and trusts only the
//! lab CA.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use shared::konst::KEYCLOAK_HTTPS_PORT;

/// Timeout of one API request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Realm holding Keycloak's administrators.
const MASTER_REALM: &str = "master";
/// Client Keycloak's administrators log in with.
const ADMIN_CLIENT_ID: &str = "admin-cli";

/// Token endpoint response, the field Sherpa reads.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// What creating a realm resource did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Created {
    Created,
    /// A resource with the same name was there already.
    AlreadyExists,
}

/// The base address of a Keycloak node.
pub fn keycloak_address(node_address: Ipv4Addr) -> String {
    format!("https://{node_address}:{KEYCLOAK_HTTPS_PORT}")
}

/// A Keycloak node's API, reached over TLS trusting only the lab CA.
#[derive(Debug)]
pub struct KeycloakClient {
    client: reqwest::Client,
    address: String,
}

impl KeycloakClient {
    pub fn new(node_address: Ipv4Addr, lab_ca_pem: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(lab_ca_pem.as_bytes())
                .context("Failed to load the lab CA certificate")?])
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("Failed to build the Keycloak API client")?;
        Ok(Self {
            client,
            address: keycloak_address(node_address),
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// Succeeds once the master realm's OpenID configuration is served.
    pub async fn ready(&self) -> Result<()> {
        let url = format!(
            "{}/realms/{MASTER_REALM}/.well-known/openid-configuration",
            self.address
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Keycloak at {} did not answer", self.address))?;
        if !response.status().is_success() {
            bail!(
                "Keycloak at {} answered {}",
                self.address,
                response.status()
            );
        }
        Ok(())
    }

    /// An access token for a master realm administrator.
    pub async fn admin_token(&self, username: &str, password: &str) -> Result<String> {
        let response = self
            .client
            .post(format!(
                "{}/realms/{MASTER_REALM}/protocol/openid-connect/token",
                self.address
            ))
            .form(&[
                ("grant_type", "password"),
                ("client_id", ADMIN_CLIENT_ID),
                ("username", username),
                ("password", password),
            ])
            .send()
            .await
            .with_context(|| format!("Keycloak at {} did not answer", self.address))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .context("Failed to read the Keycloak token response")?;
        if !status.is_success() {
            bail!(
                "Keycloak at {} refused the administrator '{username}' ({status}): {}",
                self.address,
                String::from_utf8_lossy(&body)
            );
        }
        let token: TokenResponse =
            serde_json::from_slice(&body).context("Failed to parse the Keycloak token response")?;
        Ok(token.access_token)
    }

    /// Create a resource at the admin API path `path` (under `/admin/realms`). A resource with
    /// the same name is left as it is.
    pub async fn create(&self, token: &str, path: &str, body: &Value) -> Result<Created> {
        let url = format!("{}/admin/realms{path}", self.address);
        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(body)?)
            .send()
            .await
            .with_context(|| format!("Keycloak at {} did not answer", self.address))?;
        let status = response.status();
        if status == StatusCode::CONFLICT {
            return Ok(Created::AlreadyExists);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Keycloak refused to create {url} ({status}): {body}");
        }
        Ok(Created::Created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycloak_is_on_the_management_address_over_https() {
        assert_eq!(
            keycloak_address(Ipv4Addr::new(172, 31, 1, 23)),
            "https://172.31.1.23:8443"
        );
    }

    #[test]
    fn token_responses_parse_the_access_token() {
        let token: TokenResponse = serde_json::from_str(
            r#"{"access_token":"eyJ.a.b","expires_in":60,"token_type":"Bearer"}"#,
        )
        .unwrap();
        assert_eq!(token.access_token, "eyJ.a.b");
    }
}
