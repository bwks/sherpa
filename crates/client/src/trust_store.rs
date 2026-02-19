//! Certificate trust store management for self-signed server certificates.
//!
//! This module implements a Trust-On-First-Use (TOFU) system for managing
//! server certificates. Certificates are stored in ~/.sherpa/trusted_certs/

use anyhow::{Context, Result};
use rustls::pki_types::CertificateDer;
use rustls_pemfile::certs;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Manages trusted server certificates
pub struct TrustStore {
    store_dir: PathBuf,
}

/// Information extracted from a certificate
#[derive(Debug, Clone)]
pub struct CertificateInfo {
    pub server_url: String,
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    pub valid_from: String,
    pub valid_to: String,
}

impl TrustStore {
    /// Create a new trust store instance
    ///
    /// Initializes the trust store directory at ~/.sherpa/trusted_certs/
    /// Creates the directory if it doesn't exist.
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Failed to determine home directory")?;
        let store_dir = home.join(".sherpa").join("trusted_certs");

        // Create directory if it doesn't exist
        if !store_dir.exists() {
            fs::create_dir_all(&store_dir).with_context(|| {
                format!(
                    "Failed to create trust store directory: {}",
                    store_dir.display()
                )
            })?;

            // Set restrictive permissions (owner only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o700);
                fs::set_permissions(&store_dir, perms)
                    .context("Failed to set trust store directory permissions")?;
            }

            tracing::info!("Created trust store directory: {}", store_dir.display());
        }

        Ok(Self { store_dir })
    }

    /// Get the certificate for a server URL if it exists in the trust store
    ///
    /// Returns None if no certificate is found for this server.
    pub fn get_cert(&self, server_url: &str) -> Result<Option<String>> {
        let cert_path = self.cert_path(server_url);

        if !cert_path.exists() {
            return Ok(None);
        }

        let cert_pem = fs::read_to_string(&cert_path)
            .with_context(|| format!("Failed to read certificate from: {}", cert_path.display()))?;

        // Validate it's a valid PEM format
        if !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
            anyhow::bail!(
                "Invalid certificate format in trust store: {}",
                cert_path.display()
            );
        }

        tracing::debug!("Found trusted certificate for: {}", server_url);
        Ok(Some(cert_pem))
    }

    /// Save a certificate to the trust store
    ///
    /// The certificate will be stored with restrictive permissions (owner only).
    pub fn save_cert(&self, server_url: &str, cert_pem: &str) -> Result<()> {
        // Validate PEM format
        if !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
            anyhow::bail!("Invalid certificate format: not a valid PEM certificate");
        }

        let cert_path = self.cert_path(server_url);

        // Write certificate
        fs::write(&cert_path, cert_pem)
            .with_context(|| format!("Failed to write certificate to: {}", cert_path.display()))?;

        // Set restrictive permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&cert_path, perms)
                .context("Failed to set certificate file permissions")?;
        }

        tracing::info!("Saved trusted certificate for: {}", server_url);
        tracing::debug!("Certificate path: {}", cert_path.display());

        Ok(())
    }

    /// Remove a certificate from the trust store
    ///
    /// Returns true if a certificate was removed, false if it didn't exist.
    pub fn remove_cert(&self, server_url: &str) -> Result<bool> {
        let cert_path = self.cert_path(server_url);

        if !cert_path.exists() {
            return Ok(false);
        }

        fs::remove_file(&cert_path)
            .with_context(|| format!("Failed to remove certificate: {}", cert_path.display()))?;

        tracing::info!("Removed trusted certificate for: {}", server_url);
        Ok(true)
    }

    /// Get the file path for a server's certificate
    fn cert_path(&self, server_url: &str) -> PathBuf {
        let filename = url_to_filename(server_url);
        self.store_dir.join(format!("{}.pem", filename))
    }

    /// Get the directory path for the trust store
    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    /// List all trusted certificates
    ///
    /// Returns a vector of (server_url, cert_pem) tuples for all trusted certificates.
    /// The server_url is reconstructed from the filename.
    pub fn list_all(&self) -> Result<Vec<(String, String)>> {
        let mut certs = Vec::new();

        // Read all .pem files in the trust store directory
        let entries = fs::read_dir(&self.store_dir).with_context(|| {
            format!(
                "Failed to read trust store directory: {}",
                self.store_dir.display()
            )
        })?;

        for entry in entries {
            let entry = entry.context("Failed to read directory entry")?;
            let path = entry.path();

            // Only process .pem files
            if path.extension().and_then(|s| s.to_str()) != Some("pem") {
                continue;
            }

            // Read certificate
            let cert_pem = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read certificate from: {}", path.display()))?;

            // Validate it's a valid PEM format
            if !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
                tracing::warn!("Skipping invalid certificate: {}", path.display());
                continue;
            }

            // Reconstruct server URL from filename
            // Format: host_port.pem -> wss://host:port/ws
            if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                let server_url = filename_to_url(filename);
                certs.push((server_url, cert_pem));
            }
        }

        Ok(certs)
    }
}

/// Convert a server URL to a safe filename
///
/// Extracts host and port from URL and creates a readable filename.
/// Example: wss://10.100.58.10:3030/ws -> 10.100.58.10_3030.pem
fn url_to_filename(server_url: &str) -> String {
    use url::Url;

    // Parse URL to extract host and port
    if let Ok(url) = Url::parse(server_url) {
        let host = url.host_str().unwrap_or("unknown");
        let port = url.port().or_else(|| match url.scheme() {
            "ws" => Some(80),
            "wss" => Some(443),
            _ => None,
        });

        if let Some(port) = port {
            // Replace : with _ for Windows compatibility
            // Replace other unsafe chars with _
            let safe_host = host.replace([':', '/'], "_");
            return format!("{}_{}", safe_host, port);
        }
    }

    // Fallback: use SHA-256 hash if URL parsing fails
    let mut hasher = Sha256::new();
    hasher.update(server_url.as_bytes());
    let hash = hasher.finalize();
    hex::encode(hash)
}

/// Convert a filename back to a server URL
///
/// Example: 10.100.58.10_3030 -> wss://10.100.58.10:3030/ws
fn filename_to_url(filename: &str) -> String {
    // Try to parse as host_port format
    if let Some((host, port_str)) = filename.rsplit_once('_')
        && port_str.parse::<u16>().is_ok()
    {
        // Reconstruct as wss:// URL (assume secure by default for display)
        return format!("wss://{}:{}/ws", host, port_str);
    }

    // Fallback: return filename as-is if can't parse
    format!("unknown://{}", filename)
}

/// Extract information from a certificate PEM string
pub fn extract_cert_info(server_url: &str, cert_pem: &str) -> Result<CertificateInfo> {
    // Parse PEM to DER
    let mut reader = BufReader::new(cert_pem.as_bytes());
    let cert_ders: Vec<CertificateDer> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate PEM")?;

    if cert_ders.is_empty() {
        anyhow::bail!("No certificates found in PEM data");
    }

    let cert_der = &cert_ders[0];

    // Parse the certificate using x509-parser
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der.as_ref())
        .map_err(|e| anyhow::anyhow!("Failed to parse X.509 certificate: {}", e))?;

    // Extract subject CN
    let subject = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("Unknown")
        .to_string();

    // Extract issuer CN
    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("Unknown")
        .to_string();

    // Check if self-signed
    let issuer_display = if subject == issuer {
        format!("{} (self-signed)", issuer)
    } else {
        issuer
    };

    // Extract validity dates
    let valid_from = cert.validity().not_before.to_string();
    let valid_to = cert.validity().not_after.to_string();

    // Compute SHA-256 fingerprint
    let fingerprint = compute_fingerprint(cert_der)?;

    Ok(CertificateInfo {
        server_url: server_url.to_string(),
        fingerprint,
        subject,
        issuer: issuer_display,
        valid_from,
        valid_to,
    })
}

/// Compute SHA-256 fingerprint of a certificate in colon-separated hex format
pub fn compute_fingerprint(cert_der: &CertificateDer) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(cert_der.as_ref());
    let hash = hasher.finalize();

    // Convert to colon-separated hex format: A1:B2:C3:...
    let hex_bytes: Vec<String> = hash.iter().map(|b| format!("{:02X}", b)).collect();
    Ok(hex_bytes.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TEST_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDazCCAlOgAwIBAgIUFYnDFi0kqgXhGd6WgTHZvqaJOsswDQYJKoZIhvcNAQEL
BQAwRTELMAkGA1UEBhMCQVUxEzARBgNVBAgMClNvbWUtU3RhdGUxITAfBgNVBAoM
GEludGVybmV0IFdpZGdpdHMgUHR5IEx0ZDAeFw0yNDAyMTYwMDAwMDBaFw0yNTAy
MTUyMzU5NTlaMEUxCzAJBgNVBAYTAkFVMRMwEQYDVQQIDApTb21lLVN0YXRlMSEw
HwYDVQQKDBhJbnRlcm5ldCBXaWRnaXRzIFB0eSBMdGQwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQDHg6LlvEJKCxvdv3aNi+wN8nL6yGJ9khK1rjqU8NeX
-----END CERTIFICATE-----"#;

    #[test]
    fn test_url_to_filename() {
        let url1 = "wss://192.168.1.100:3030/ws";
        let url2 = "wss://192.168.1.100:3030/ws";
        let url3 = "wss://10.0.0.5:3030/ws";

        let filename1 = url_to_filename(url1);
        let filename2 = url_to_filename(url2);
        let filename3 = url_to_filename(url3);

        // Same URL should produce same filename
        assert_eq!(filename1, filename2);

        // Different URLs should produce different filenames
        assert_ne!(filename1, filename3);

        // Filename should be valid hex
        assert!(filename1.chars().all(|c| c.is_ascii_hexdigit()));

        // Filename should be 64 characters (SHA-256 hex)
        assert_eq!(filename1.len(), 64);
    }

    #[test]
    fn test_trust_store_new() {
        let temp_home = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_home.path());

        let trust_store = TrustStore::new().unwrap();

        // Check directory was created
        assert!(trust_store.store_dir().exists());

        // Check permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(trust_store.store_dir()).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn test_save_and_get_cert() {
        let temp_home = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_home.path());

        let trust_store = TrustStore::new().unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        // Initially should return None
        let result = trust_store.get_cert(server_url).unwrap();
        assert!(result.is_none());

        // Save certificate
        trust_store.save_cert(server_url, TEST_CERT_PEM).unwrap();

        // Now should return the certificate
        let result = trust_store.get_cert(server_url).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), TEST_CERT_PEM);
    }

    #[test]
    fn test_remove_cert() {
        let temp_home = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_home.path());

        let trust_store = TrustStore::new().unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        // Save certificate
        trust_store.save_cert(server_url, TEST_CERT_PEM).unwrap();

        // Remove it
        let removed = trust_store.remove_cert(server_url).unwrap();
        assert!(removed);

        // Should no longer exist
        let result = trust_store.get_cert(server_url).unwrap();
        assert!(result.is_none());

        // Removing again should return false
        let removed = trust_store.remove_cert(server_url).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_invalid_pem_format() {
        let temp_home = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_home.path());

        let trust_store = TrustStore::new().unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        // Try to save invalid PEM
        let result = trust_store.save_cert(server_url, "not a valid pem");
        assert!(result.is_err());
    }
}
