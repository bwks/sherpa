//! Certificate trust store management for self-signed server certificates.
//!
//! This module implements a Trust-On-First-Use (TOFU) system for managing
//! server certificates. Certificates are stored in ~/.`sherpa/trusted_certs`/

use anyhow::{Context, Result, anyhow};
use rustls::pki_types::CertificateDer;
use rustls_pemfile::certs;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::BufReader;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use url::Url;

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
    /// Whether the certificate names itself as its issuer.
    pub self_signed: bool,
    pub valid_from: String,
    pub valid_to: String,
}

impl TrustStore {
    /// Open the user's trust store, `~/.sherpa/trusted_certs/`, creating it if needed.
    ///
    /// # Errors
    ///
    /// Fails when the home directory is unknown or the store cannot be created.
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Failed to determine home directory")?;
        Self::at(home.join(".sherpa").join("trusted_certs"))
    }

    /// Open the trust store in `store_dir`, creating it owner-only if it does not exist.
    ///
    /// # Errors
    ///
    /// Fails when the folder cannot be created or its mode set.
    pub fn at(store_dir: PathBuf) -> Result<Self> {
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
            fs::set_permissions(&store_dir, fs::Permissions::from_mode(0o700))
                .context("Failed to set trust store directory permissions")?;

            tracing::info!("Created trust store directory: {}", store_dir.display());
        }

        Ok(Self { store_dir })
    }

    /// Get the certificate for a server URL if it exists in the trust store
    ///
    /// Returns None if no certificate is found for this server.
    ///
    /// # Errors
    ///
    /// Fails when the saved certificate cannot be read or does not parse.
    pub fn get_cert(&self, server_url: &str) -> Result<Option<String>> {
        let cert_path = self.cert_path(server_url);

        if !cert_path.exists() {
            return Ok(None);
        }

        let cert_pem = fs::read_to_string(&cert_path)
            .with_context(|| format!("Failed to read certificate from: {}", cert_path.display()))?;

        parse_certificate_pem(&cert_pem).with_context(|| {
            format!(
                "Invalid certificate in trust store: {}",
                cert_path.display()
            )
        })?;

        tracing::debug!("Found trusted certificate for: {}", server_url);
        Ok(Some(cert_pem))
    }

    /// Save a certificate to the trust store
    ///
    /// The certificate will be stored with restrictive permissions (owner only).
    ///
    /// # Errors
    ///
    /// Fails when `cert_pem` is not a certificate, or it cannot be written.
    pub fn save_cert(&self, server_url: &str, cert_pem: &str) -> Result<()> {
        parse_certificate_pem(cert_pem).context("Invalid certificate")?;

        let cert_path = self.cert_path(server_url);

        // Write certificate
        fs::write(&cert_path, cert_pem)
            .with_context(|| format!("Failed to write certificate to: {}", cert_path.display()))?;

        // Set restrictive permissions
        #[cfg(unix)]
        fs::set_permissions(&cert_path, fs::Permissions::from_mode(0o600))
            .context("Failed to set certificate file permissions")?;

        tracing::info!("Saved trusted certificate for: {}", server_url);
        tracing::debug!("Certificate path: {}", cert_path.display());

        Ok(())
    }

    /// Remove a certificate from the trust store
    ///
    /// Returns true if a certificate was removed, false if it didn't exist.
    ///
    /// # Errors
    ///
    /// Fails when the certificate file exists but cannot be removed.
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
        self.store_dir.join(format!("{filename}.pem"))
    }

    /// Get the directory path for the trust store
    #[must_use]
    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    /// List all trusted certificates
    ///
    /// Returns a vector of (`server_url`, `cert_pem`) tuples for all trusted certificates.
    /// The `server_url` is reconstructed from the filename.
    ///
    /// # Errors
    ///
    /// Fails when the store or one of its files cannot be read; a file that does not parse is skipped.
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

            if let Err(error) = parse_certificate_pem(&cert_pem) {
                tracing::warn!(path = %path.display(), error = format!("{error:#}"), "Skipping invalid certificate");
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
/// Example: <wss://10.100.58.10:3030/ws> -> `10.100.58.10_3030.pem`
fn url_to_filename(server_url: &str) -> String {
    // Parse URL to extract host and port
    if let Ok(url) = Url::parse(server_url) {
        let host = url.host_str().unwrap_or("unknown");
        if let Some(port) = url.port_or_known_default() {
            // Replace : with _ for Windows compatibility
            // Replace other unsafe chars with _
            let safe_host = host.replace([':', '/'], "_");
            return format!("{safe_host}_{port}");
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
/// Example: `10.100.58.10_3030` -> <wss://10.100.58.10:3030/ws>
fn filename_to_url(filename: &str) -> String {
    // Try to parse as host_port format
    if let Some((host, port_str)) = filename.rsplit_once('_')
        && port_str.parse::<u16>().is_ok()
    {
        // Reconstruct as wss:// URL (assume secure by default for display)
        return format!("wss://{host}:{port_str}/ws");
    }

    // Fallback: return filename as-is if can't parse
    format!("unknown://{filename}")
}

/// The first certificate in a PEM string, checked to be an X.509 certificate.
///
/// # Errors
///
/// Fails when the text holds no PEM certificate, or the first one is not valid X.509.
pub fn parse_certificate_pem(cert_pem: &str) -> Result<CertificateDer<'static>> {
    let mut reader = BufReader::new(cert_pem.as_bytes());
    let cert_der = certs(&mut reader)
        .next()
        .context("No certificates found in PEM data")?
        .context("Failed to parse certificate PEM")?;
    x509_parser::parse_x509_certificate(cert_der.as_ref())
        .map_err(|error| anyhow!("Failed to parse X.509 certificate: {error}"))?;
    Ok(cert_der)
}

/// Extract information from a certificate PEM string
///
/// # Errors
///
/// Fails when `cert_pem` holds no valid certificate.
pub fn extract_cert_info(server_url: &str, cert_pem: &str) -> Result<CertificateInfo> {
    let cert_der = &parse_certificate_pem(cert_pem)?;
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der.as_ref())
        .map_err(|error| anyhow!("Failed to parse X.509 certificate: {error}"))?;

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

    let self_signed = subject == issuer;

    // Extract validity dates
    let valid_from = cert.validity().not_before.to_string();
    let valid_to = cert.validity().not_after.to_string();

    // Compute SHA-256 fingerprint
    let fingerprint = compute_fingerprint(cert_der)?;

    Ok(CertificateInfo {
        server_url: server_url.to_string(),
        fingerprint,
        subject,
        issuer,
        self_signed,
        valid_from,
        valid_to,
    })
}

/// Compute SHA-256 fingerprint of a certificate in colon-separated hex format
///
/// # Errors
///
/// Never fails today; the `Result` is kept for callers.
pub fn compute_fingerprint(cert_der: &CertificateDer) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(cert_der.as_ref());
    let hash = hasher.finalize();

    // Convert to colon-separated hex format: A1:B2:C3:...
    let hex_bytes: Vec<String> = hash.iter().map(|b| format!("{b:02X}")).collect();
    Ok(hex_bytes.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const SELF_SIGNED: &str = include_str!("testdata/self_signed.crt");

    #[test]
    fn a_self_signed_certificate_is_flagged_and_keeps_its_issuer() {
        let info = extract_cert_info("wss://localhost:3030/ws", SELF_SIGNED).unwrap();
        assert_eq!(info.subject, "Sherpa Test");
        assert_eq!(info.issuer, "Sherpa Test");
        assert!(info.self_signed);
    }

    /// The start of a certificate, cut short: it has the PEM markers but does not parse.
    const TRUNCATED_CERT_PEM: &str = r"-----BEGIN CERTIFICATE-----
MIIDazCCAlOgAwIBAgIUFYnDFi0kqgXhGd6WgTHZvqaJOsswDQYJKoZIhvcNAQEL
BQAwRTELMAkGA1UEBhMCQVUxEzARBgNVBAgMClNvbWUtU3RhdGUxITAfBgNVBAoM
GEludGVybmV0IFdpZGdpdHMgUHR5IEx0ZDAeFw0yNDAyMTYwMDAwMDBaFw0yNTAy
MTUyMzU5NTlaMEUxCzAJBgNVBAYTAkFVMRMwEQYDVQQIDApTb21lLVN0YXRlMSEw
HwYDVQQKDBhJbnRlcm5ldCBXaWRnaXRzIFB0eSBMdGQwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQDHg6LlvEJKCxvdv3aNi+wN8nL6yGJ9khK1rjqU8NeX
-----END CERTIFICATE-----";

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

        // Filename should be in host_port format
        assert_eq!(filename1, "192.168.1.100_3030");
        assert_eq!(filename3, "10.0.0.5_3030");
    }

    #[test]
    fn test_trust_store_new() {
        let dir = TempDir::new().unwrap();
        let trust_store = TrustStore::at(dir.path().join("trusted_certs")).unwrap();

        assert!(trust_store.store_dir().exists());
        #[cfg(unix)]
        {
            let mode = fs::metadata(trust_store.store_dir())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn test_save_and_get_cert() {
        let dir = TempDir::new().unwrap();
        let trust_store = TrustStore::at(dir.path().to_path_buf()).unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        assert!(trust_store.get_cert(server_url).unwrap().is_none());
        trust_store.save_cert(server_url, SELF_SIGNED).unwrap();
        assert_eq!(
            trust_store.get_cert(server_url).unwrap().as_deref(),
            Some(SELF_SIGNED)
        );
        assert_eq!(
            trust_store.list_all().unwrap(),
            [(server_url.to_string(), SELF_SIGNED.to_string())]
        );
    }

    #[test]
    fn test_remove_cert() {
        let dir = TempDir::new().unwrap();
        let trust_store = TrustStore::at(dir.path().to_path_buf()).unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        trust_store.save_cert(server_url, SELF_SIGNED).unwrap();
        assert!(trust_store.remove_cert(server_url).unwrap());
        assert!(trust_store.get_cert(server_url).unwrap().is_none());
        assert!(!trust_store.remove_cert(server_url).unwrap());
    }

    #[test]
    fn a_certificate_that_does_not_parse_is_refused() {
        let dir = TempDir::new().unwrap();
        let trust_store = TrustStore::at(dir.path().to_path_buf()).unwrap();
        let server_url = "wss://test.example.com:3030/ws";

        assert!(
            trust_store
                .save_cert(server_url, "not a valid pem")
                .is_err()
        );
        assert!(
            trust_store
                .save_cert(server_url, TRUNCATED_CERT_PEM)
                .is_err()
        );
    }

    #[test]
    fn a_url_without_a_port_is_stored_under_its_scheme_default() {
        assert_eq!(
            url_to_filename("wss://sherpa.example/ws"),
            "sherpa.example_443"
        );
    }
}
