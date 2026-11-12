//! Certificate management commands for TOFU system
//!
//! Provides CLI commands to list, show, trust, and delete trusted server certificates.

use anyhow::{Context, Result};
use url::Url;

use shared::konst::SHERPA_SERVER_WS_PORT;
use shared::tls::{
    TrustStore, extract_cert_info, fetch_server_certificate, print_certificate_info,
};
use shared::util::{Emoji, confirm};

use crate::table::{CertificateTableInfo, render_certificates_table};

/// List all trusted certificates
pub fn cert_list() -> Result<()> {
    let trust_store = TrustStore::new().context("Failed to initialize trust store")?;
    let certs = trust_store
        .list_all()
        .context("Failed to list certificates")?;

    if certs.is_empty() {
        println!("No trusted certificates found.");
        println!();
        println!(
            "Trust store location: {}",
            trust_store.store_dir().display()
        );
        return Ok(());
    }

    let cert_info: Vec<CertificateTableInfo> = certs
        .iter()
        .map(|(server_url, cert_pem)| {
            // Extract clean server display (just host:port)
            let server_display = extract_server_display(server_url);

            // Extract certificate info
            match extract_cert_info(server_url, cert_pem) {
                Ok(info) => {
                    // Parse valid_to date to extract just the date part
                    let valid_until = info
                        .valid_to
                        .split_whitespace()
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(" ");

                    CertificateTableInfo {
                        server: server_display,
                        subject: info.subject.clone(),
                        valid_until,
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse certificate for {}: {}", server_url, e);
                    CertificateTableInfo {
                        server: server_display,
                        subject: "<parse error>".to_string(),
                        valid_until: String::new(),
                    }
                }
            }
        })
        .collect();

    let table = render_certificates_table(&cert_info);

    println!("{table}");
    println!();
    println!(
        "Total: {} trusted certificate{}",
        certs.len(),
        if certs.len() == 1 { "" } else { "s" }
    );
    println!();
    println!(
        "Trust store location: {}",
        trust_store.store_dir().display()
    );

    Ok(())
}

/// Show detailed information about a specific certificate
pub fn cert_show(server_url: &str) -> Result<()> {
    let trust_store = TrustStore::new().context("Failed to initialize trust store")?;

    // Normalize the input to a full URL
    let normalized_url = normalize_server_url(server_url)?;

    // Get certificate from trust store
    let cert_pem = trust_store
        .get_cert(&normalized_url)?
        .with_context(|| format!("No trusted certificate found for: {server_url}"))?;

    // Extract certificate info
    let cert_info = extract_cert_info(&normalized_url, &cert_pem)
        .context("Failed to parse certificate information")?;

    println!();
    print_certificate_info(&cert_info);

    // Show file location
    println!("Certificate store: {}", trust_store.store_dir().display());

    Ok(())
}

/// Manually fetch and trust a certificate
pub async fn cert_trust(server_url: &str, http_port: u16) -> Result<()> {
    let trust_store = TrustStore::new().context("Failed to initialize trust store")?;

    // Normalize the input to a full URL
    let normalized_url = normalize_server_url(server_url)?;

    // Check if already trusted
    if trust_store.get_cert(&normalized_url)?.is_some() {
        println!(
            "{} Certificate already trusted for: {server_url}",
            Emoji::Warning
        );
        if !confirm("Do you want to re-fetch and update it?")? {
            println!("Operation cancelled.");
            return Ok(());
        }
    }

    println!("Fetching certificate from {normalized_url}...");
    println!();

    // Fetch certificate from server
    let cert_pem = fetch_server_certificate(&normalized_url, http_port)
        .await
        .context("Failed to fetch server certificate")?;

    // Extract and display certificate info
    let cert_info = extract_cert_info(&normalized_url, &cert_pem)
        .context("Failed to parse certificate information")?;

    print_certificate_info(&cert_info);
    println!(
        "{} WARNING: Verify the fingerprint matches what you expect to prevent",
        Emoji::Warning
    );
    println!("   man-in-the-middle attacks.");
    println!();
    if !confirm("Do you trust this certificate?")? {
        println!("Certificate not trusted. Operation cancelled.");
        return Ok(());
    }

    // Save to trust store
    trust_store
        .save_cert(&normalized_url, &cert_pem)
        .context("Failed to save certificate to trust store")?;

    println!("{} Certificate saved to trust store", Emoji::Success);
    println!();

    Ok(())
}

/// Delete a trusted certificate
pub fn cert_delete(server_url: &str) -> Result<()> {
    let trust_store = TrustStore::new().context("Failed to initialize trust store")?;

    // Normalize the input to a full URL
    let normalized_url = normalize_server_url(server_url)?;

    // Check if certificate exists
    let Some(cert_pem) = trust_store.get_cert(&normalized_url)? else {
        println!("No trusted certificate found for: {server_url}");
        return Ok(());
    };

    // Show certificate info before deletion
    if let Ok(cert_info) = extract_cert_info(&normalized_url, &cert_pem) {
        println!();
        println!(
            "Certificate found for: {}",
            extract_server_display(&normalized_url)
        );
        println!();
        println!("  Subject:     {}", cert_info.subject);
        println!("  Fingerprint: {}", cert_info.fingerprint);
        println!();
    }

    if !confirm("Are you sure you want to delete this certificate?")? {
        println!("Operation cancelled.");
        return Ok(());
    }

    // Remove from trust store
    trust_store
        .remove_cert(&normalized_url)
        .context("Failed to remove certificate from trust store")?;

    println!("{} Certificate deleted from trust store", Emoji::Success);
    println!();
    println!("Note: You will be prompted to trust the certificate again on next connection.");

    Ok(())
}

/// Extract clean server display name (host:port) from server URL
fn extract_server_display(server_url: &str) -> String {
    // If it's a legacy certificate with unknown:// prefix, show as such
    if server_url.starts_with("unknown://") {
        return "<legacy>".to_string();
    }

    // Try to parse as URL
    if let Ok(url) = Url::parse(server_url) {
        let host = url.host_str().unwrap_or("unknown");

        // Get port (explicit or default based on scheme)
        let port = url.port().or_else(|| match url.scheme() {
            "ws" => Some(80),
            "wss" => Some(443),
            _ => None,
        });

        if let Some(port) = port {
            // Show non-standard ports explicitly
            let standard_port = match url.scheme() {
                "ws" => 80,
                "wss" => 443,
                _ => 0,
            };

            if port != standard_port {
                return format!("{host}:{port}");
            }
            return host.to_string();
        }
        return host.to_string();
    }

    // Fallback: return as-is
    server_url.to_string()
}

/// The WebSocket URL of a server given as a host, `host:port` or a `ws://` or `wss://` URL,
/// such as `10.100.58.10`, `10.100.58.10:3030` or `wss://10.100.58.10:3030/ws`. A host without
/// a port gets the Sherpa server's port, as the client connects to it, so the URL names the same
/// trust store entry as a connection does.
fn normalize_server_url(input: &str) -> Result<String> {
    if let Ok(url) = Url::parse(input)
        && matches!(url.scheme(), "ws" | "wss")
    {
        return Ok(input.to_owned());
    }
    // A scheme with no default port, so the port is only set when the input names one.
    let address = Url::parse(&format!("sherpa://{input}"))
        .ok()
        .filter(|url| matches!(url.path(), "" | "/") && url.query().is_none())
        .with_context(|| {
            format!("Server '{input}' must be a host, host:port, or a ws:// or wss:// URL")
        })?;
    let host = address
        .host_str()
        .with_context(|| format!("Server '{input}' names no host"))?;
    let port = address.port().unwrap_or(SHERPA_SERVER_WS_PORT);
    Ok(format!("wss://{host}:{port}/ws"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_uses_the_sherpa_server_port() {
        assert_eq!(
            normalize_server_url("10.0.0.1").unwrap(),
            "wss://10.0.0.1:3030/ws"
        );
        assert_eq!(
            normalize_server_url("[::1]").unwrap(),
            "wss://[::1]:3030/ws"
        );
    }

    #[test]
    fn an_explicit_port_is_kept() {
        assert_eq!(
            normalize_server_url("10.0.0.1:3030").unwrap(),
            "wss://10.0.0.1:3030/ws"
        );
        assert_eq!(
            normalize_server_url("example.com:443").unwrap(),
            "wss://example.com:443/ws"
        );
        assert_eq!(
            normalize_server_url("[::1]:4000").unwrap(),
            "wss://[::1]:4000/ws"
        );
    }

    #[test]
    fn a_websocket_url_is_used_as_given() {
        assert_eq!(
            normalize_server_url("wss://h:3030/ws").unwrap(),
            "wss://h:3030/ws"
        );
        assert_eq!(
            normalize_server_url("ws://h:3030/ws").unwrap(),
            "ws://h:3030/ws"
        );
    }

    #[test]
    fn other_urls_are_refused() {
        for input in ["https://h:3030", "h:3030/path", ""] {
            assert!(normalize_server_url(input).is_err(), "{input}");
        }
    }
}
