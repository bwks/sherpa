use anyhow::{Context, Result};
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName};
use shared::data::ServerConnection;
use shared::util::{term_msg_surround, term_msg_underline};
use std::fs::File;
use std::io::{self, BufReader, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;

use crate::cert_fetch::fetch_server_certificate;
use crate::trust_store::{TrustStore, extract_cert_info};

/// Builds TLS configuration for WebSocket client
pub struct TlsConfigBuilder {
    _validate_certs: bool,
    ca_cert_path: Option<String>,
    insecure: bool,
}

impl TlsConfigBuilder {
    /// Create a new TLS config builder from server connection settings
    pub fn new(server_conn: &ServerConnection) -> Self {
        Self {
            _validate_certs: server_conn.validate_certs,
            ca_cert_path: server_conn.ca_cert_path.clone(),
            insecure: server_conn.insecure,
        }
    }

    /// Build rustls ClientConfig
    pub fn _build(&self) -> Result<Arc<ClientConfig>> {
        if self.insecure {
            tracing::warn!("INSECURE MODE: TLS certificate validation is DISABLED");
            return Ok(Arc::new(Self::build_insecure()));
        }

        let mut root_store = RootCertStore::empty();

        // Add custom CA certificate if provided
        if let Some(ref ca_path) = self.ca_cert_path {
            tracing::info!("Using custom CA certificate: {}", ca_path);
            let ca_cert = Self::load_ca_cert(Path::new(ca_path))
                .context("Failed to load custom CA certificate")?;
            root_store
                .add(ca_cert)
                .context("Failed to add custom CA certificate to root store")?;
        } else if self._validate_certs {
            // Use webpki roots (Mozilla's CA certificates)
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            tracing::debug!("Using webpki system CA certificates for validation");
        }

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Arc::new(config))
    }

    /// Build rustls ClientConfig with Trust-On-First-Use flow
    ///
    /// This method implements an interactive trust flow for self-signed certificates:
    /// 1. If insecure mode or custom CA path is set, use existing logic
    /// 2. Check if certificate exists in trust store
    /// 3. If not, fetch certificate from server and prompt user to trust it
    /// 4. Save to trust store if user confirms
    pub async fn build_with_trust_flow(&self, server_url: &str) -> Result<Arc<ClientConfig>> {
        // If insecure mode, bypass all validation
        if self.insecure {
            tracing::warn!("INSECURE MODE: TLS certificate validation is DISABLED");
            return Ok(Arc::new(Self::build_insecure()));
        }

        // If custom CA path provided, use existing logic
        if let Some(ref ca_path) = self.ca_cert_path {
            tracing::info!("Using custom CA certificate: {}", ca_path);
            let ca_cert = Self::load_ca_cert(Path::new(ca_path))
                .context("Failed to load custom CA certificate")?;

            let mut root_store = RootCertStore::empty();
            root_store
                .add(ca_cert)
                .context("Failed to add custom CA certificate to root store")?;

            let config = ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();

            return Ok(Arc::new(config));
        }

        // Initialize trust store
        let trust_store =
            TrustStore::new().context("Failed to initialize certificate trust store")?;

        // Check if certificate already exists in trust store
        let cert_pem = match trust_store.get_cert(server_url)? {
            Some(existing_cert) => {
                tracing::info!("Using trusted certificate for: {}", server_url);
                existing_cert
            }
            None => {
                // Certificate not in trust store - fetch from server
                tracing::info!("Certificate not found in trust store, fetching from server...");

                let fetched_cert = fetch_server_certificate(server_url)
                    .await
                    .context("Failed to fetch server certificate")?;

                // Extract certificate info for display
                let cert_info = extract_cert_info(server_url, &fetched_cert)
                    .context("Failed to parse certificate information")?;

                // Display certificate information
                display_certificate_info(&cert_info)?;

                // Prompt user to trust the certificate
                if !prompt_trust_certificate()? {
                    anyhow::bail!(
                        "Certificate not trusted by user.\n\n\
                         Connection aborted. To bypass certificate validation (not recommended),\n\
                         use the --insecure flag."
                    );
                }

                // Save certificate to trust store
                trust_store
                    .save_cert(server_url, &fetched_cert)
                    .context("Failed to save certificate to trust store")?;

                println!("✅ Certificate saved to trust store");
                println!();

                fetched_cert
            }
        };

        // Load the certificate into a root store
        let mut root_store = RootCertStore::empty();
        let cert =
            Self::load_cert_from_pem(&cert_pem).context("Failed to parse trusted certificate")?;

        root_store
            .add(cert)
            .context("Failed to add trusted certificate to root store")?;

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Arc::new(config))
    }

    /// Load a certificate from PEM string
    fn load_cert_from_pem(pem: &str) -> Result<CertificateDer<'static>> {
        let mut reader = BufReader::new(pem.as_bytes());
        let mut certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse certificate from PEM")?;

        if certs.is_empty() {
            anyhow::bail!("No certificates found in PEM data");
        }

        Ok(certs.remove(0))
    }

    /// Load a CA certificate from file
    fn load_ca_cert(path: &Path) -> Result<CertificateDer<'static>> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open CA certificate file: {}", path.display()))?;
        let mut reader = BufReader::new(file);

        let mut certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse CA certificate from PEM file")?;

        if certs.is_empty() {
            anyhow::bail!("No certificates found in CA file: {}", path.display());
        }

        Ok(certs.remove(0))
    }

    /// Create insecure config that skips verification (DEV/TEST ONLY)
    fn build_insecure() -> ClientConfig {
        let mut config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth();

        // Allow using SSLKEYLOGFILE for debugging
        config.key_log = Arc::new(rustls::KeyLogFile::new());

        config
    }
}

/// Custom certificate verifier that accepts anything (INSECURE - for development only)
#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Accept any certificate without validation
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// Display certificate information to the user in a formatted way
fn display_certificate_info(cert_info: &crate::trust_store::CertificateInfo) -> Result<()> {
    println!();
    term_msg_surround("Certificate Information");
    println!();
    println!("  Server:      {}", cert_info.server_url);
    println!("  Subject:     {}", cert_info.subject);

    // Show if self-signed
    if cert_info.subject == cert_info.issuer {
        println!("  Issuer:      {} (self-signed)", cert_info.issuer);
    } else {
        println!("  Issuer:      {}", cert_info.issuer);
    }

    println!("  Valid From:  {}", cert_info.valid_from);
    println!("  Valid To:    {}", cert_info.valid_to);
    println!();
    term_msg_underline("Certificate Fingerprint (SHA-256)");
    println!("  {}", cert_info.fingerprint);
    println!();
    println!("⚠️  WARNING: This is the first time connecting to this server.");
    println!("   Verify the fingerprint matches what you expect to prevent");
    println!("   man-in-the-middle attacks.");
    println!();

    Ok(())
}

/// Prompt the user to trust a certificate
///
/// Returns true if the user confirms trust, false otherwise.
fn prompt_trust_certificate() -> Result<bool> {
    // Check if running in interactive mode
    if !is_interactive() {
        anyhow::bail!(
            "Cannot prompt for certificate trust in non-interactive mode.\n\n\
             The server certificate is not in your trust store and user confirmation\n\
             is required. This environment does not have an interactive terminal.\n\n\
             Options:\n\
             1. Run 'sherpa cert trust <server-url>' from an interactive terminal\n\
             2. Use --insecure flag to skip validation (NOT RECOMMENDED for production)"
        );
    }

    print!("Do you trust this certificate? (yes/no): ");
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut response = String::new();
    io::stdin()
        .read_line(&mut response)
        .context("Failed to read user input")?;

    let response = response.trim().to_lowercase();
    Ok(response == "yes" || response == "y")
}

/// Check if running in an interactive terminal
fn is_interactive() -> bool {
    io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_insecure_config() {
        let server_conn = ServerConnection {
            url: None,
            timeout_secs: 3,
            validate_certs: false,
            ca_cert_path: None,
            insecure: true,
        };

        let builder = TlsConfigBuilder::new(&server_conn);
        let result = builder._build();

        assert!(result.is_ok());
    }

    #[test]
    fn test_build_with_system_certs() {
        let server_conn = ServerConnection {
            url: None,
            timeout_secs: 3,
            validate_certs: true,
            ca_cert_path: None,
            insecure: false,
        };

        let builder = TlsConfigBuilder::new(&server_conn);
        let result = builder._build();

        assert!(result.is_ok());
    }
}
