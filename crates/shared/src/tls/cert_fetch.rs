//! Certificate fetching from server via HTTP endpoint.
//!
//! This module provides functionality to download server certificates
//! via an insecure HTTP connection before establishing trust.

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use std::time::Duration;
use url::Url;

use crate::tls::trust_store::parse_certificate_pem;

/// Fetch server certificate from the /cert endpoint via HTTP
///
/// # Arguments
/// * `server_url` - The WebSocket server URL (ws:// or wss://)
/// * `http_port` - The server's HTTP port, which serves the /cert endpoint.
///
/// # Returns
/// The certificate in PEM format
///
/// # Errors
/// - Network errors (connection failed, timeout, etc.)
/// - Server returned non-200 status
/// - Invalid PEM format received
pub async fn fetch_server_certificate(server_url: &str, http_port: u16) -> Result<String> {
    // Parse the WebSocket URL to extract host
    let ws_url =
        Url::parse(server_url).with_context(|| format!("Invalid server URL: {server_url}"))?;

    let host = ws_url.host_str().context("Server URL missing host")?;

    // Construct HTTP URL for certificate endpoint
    let cert_url = format!("http://{host}:{http_port}/cert");
    tracing::debug!("Fetching certificate from: {}", cert_url);

    // Create HTTP client with timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client")?;

    // Fetch certificate
    let response =
        client.get(&cert_url).send().await.with_context(|| {
            format!("Unable to reach server at {cert_url}. Is the server running?")
        })?;

    // Check status code
    match response.status() {
        StatusCode::OK => {
            // Success - read certificate
            let cert_pem = response
                .text()
                .await
                .context("Failed to read certificate response body")?;

            parse_certificate_pem(&cert_pem).context("Server returned an invalid certificate")?;

            tracing::info!("Successfully fetched certificate from server");
            Ok(cert_pem)
        }
        StatusCode::NOT_FOUND => {
            anyhow::bail!(
                "Server doesn't support certificate download.\n\
                 The /cert endpoint is not available. This may not be a Sherpa server,\n\
                 or it's running an older version without certificate download support.\n\n\
                 Use --insecure to bypass certificate validation (not recommended)."
            )
        }
        StatusCode::SERVICE_UNAVAILABLE => {
            anyhow::bail!(
                "Server TLS is disabled.\n\
                 The server is not using TLS, so no certificate is available.\n\
                 You should connect using ws:// instead of wss://."
            )
        }
        status => {
            // Try to get error message from response
            let error_msg = response.text().await.unwrap_or_default();

            anyhow::bail!("Server returned error status {status}:\n{error_msg}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    const SELF_SIGNED: &str = include_str!("testdata/self_signed.crt");

    /// Answer one request on a free local port with the test certificate, and return the port.
    fn serve_certificate_once() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{SELF_SIGNED}",
                SELF_SIGNED.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        port
    }

    #[tokio::test]
    async fn certificate_trust_008_the_certificate_is_fetched_from_the_servers_http_port() {
        let port = serve_certificate_once();
        let pem = fetch_server_certificate("wss://127.0.0.1:3030/ws", port)
            .await
            .unwrap();
        assert_eq!(pem, SELF_SIGNED);
    }
}
