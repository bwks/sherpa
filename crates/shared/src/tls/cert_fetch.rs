//! Certificate fetching from server via HTTP endpoint.
//!
//! This module provides functionality to download server certificates
//! via an insecure HTTP connection before establishing trust.

use anyhow::{Context, Result};
use std::time::Duration;
use url::Url;

/// Fetch server certificate from the /cert endpoint via HTTP
///
/// # Arguments
/// * `server_url` - The WebSocket server URL (ws:// or wss://)
///
/// # Returns
/// The certificate in PEM format
///
/// # Errors
/// - Network errors (connection failed, timeout, etc.)
/// - Server returned non-200 status
/// - Invalid PEM format received
pub async fn fetch_server_certificate(server_url: &str) -> Result<String> {
    // Parse the WebSocket URL to extract host and port
    let ws_url =
        Url::parse(server_url).with_context(|| format!("Invalid server URL: {}", server_url))?;

    let host = ws_url.host_str().context("Server URL missing host")?;

    let port = ws_url
        .port()
        .or_else(|| {
            // Default ports based on scheme
            match ws_url.scheme() {
                "ws" => Some(80),
                "wss" => Some(443),
                _ => None,
            }
        })
        .context("Unable to determine server port")?;

    // For WSS connections, the HTTP /cert endpoint is on port + 1
    // For WS connections, use the same port
    let http_port = if ws_url.scheme() == "wss" {
        port + 1
    } else {
        port
    };

    // Construct HTTP URL for certificate endpoint
    let cert_url = format!("http://{}:{}/cert", host, http_port);
    tracing::debug!("Fetching certificate from: {}", cert_url);

    // Create HTTP client with timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client")?;

    // Fetch certificate
    let response = client.get(&cert_url).send().await.with_context(|| {
        format!(
            "Unable to reach server at {}. Is the server running?",
            cert_url
        )
    })?;

    // Check status code
    match response.status() {
        reqwest::StatusCode::OK => {
            // Success - read certificate
            let cert_pem = response
                .text()
                .await
                .context("Failed to read certificate response body")?;

            // Validate it's a valid PEM format
            if !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
                anyhow::bail!(
                    "Server returned invalid certificate format. \
                     The response doesn't appear to be a valid PEM certificate."
                );
            }

            tracing::info!("Successfully fetched certificate from server");
            Ok(cert_pem)
        }
        reqwest::StatusCode::NOT_FOUND => {
            anyhow::bail!(
                "Server doesn't support certificate download.\n\
                 The /cert endpoint is not available. This may not be a Sherpa server,\n\
                 or it's running an older version without certificate download support.\n\n\
                 Use --insecure to bypass certificate validation (not recommended)."
            )
        }
        reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            anyhow::bail!(
                "Server TLS is disabled.\n\
                 The server is not using TLS, so no certificate is available.\n\
                 You should connect using ws:// instead of wss://."
            )
        }
        status => {
            // Try to get error message from response
            let error_msg = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            anyhow::bail!("Server returned error status {}:\n{}", status, error_msg)
        }
    }
}

/// Parse a WebSocket URL and extract host:port for certificate lookup
///
/// # Arguments
/// * `server_url` - The WebSocket server URL (ws:// or wss://)
///
/// # Returns
/// A string in the format "host:port" for use in trust store lookups
pub fn _parse_server_address(server_url: &str) -> Result<String> {
    let ws_url =
        Url::parse(server_url).with_context(|| format!("Invalid server URL: {}", server_url))?;

    let host = ws_url.host_str().context("Server URL missing host")?;

    let port = ws_url
        .port()
        .or_else(|| match ws_url.scheme() {
            "ws" => Some(80),
            "wss" => Some(443),
            _ => None,
        })
        .context("Unable to determine server port")?;

    Ok(format!("{}:{}", host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_server_address() {
        // WSS with explicit port
        let addr = _parse_server_address("wss://192.168.1.100:3030/ws").unwrap();
        assert_eq!(addr, "192.168.1.100:3030");

        // WS with explicit port
        let addr = _parse_server_address("ws://localhost:3030/ws").unwrap();
        assert_eq!(addr, "localhost:3030");

        // WSS with default port
        let addr = _parse_server_address("wss://example.com/ws").unwrap();
        assert_eq!(addr, "example.com:443");

        // WS with default port
        let addr = _parse_server_address("ws://example.com/ws").unwrap();
        assert_eq!(addr, "example.com:80");

        // Hostname with port
        let addr = _parse_server_address("wss://sherpa.example.com:8443/ws").unwrap();
        assert_eq!(addr, "sherpa.example.com:8443");
    }

    #[test]
    fn test_parse_invalid_url() {
        let result = _parse_server_address("not a url");
        assert!(result.is_err());

        let result = _parse_server_address("http://example.com");
        assert!(result.is_err()); // HTTP scheme is not handled, only ws/wss
    }

    #[tokio::test]
    async fn test_fetch_certificate_invalid_url() {
        let result = fetch_server_certificate("not a valid url").await;
        assert!(result.is_err());
    }

    // Note: Integration tests with real server would go in tests/ directory
}
