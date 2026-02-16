use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::ClientConfig;
use rustls::RootCertStore;
use shared::data::ServerConnection;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

/// Builds TLS configuration for WebSocket client
pub struct TlsConfigBuilder {
    validate_certs: bool,
    ca_cert_path: Option<String>,
    insecure: bool,
}

impl TlsConfigBuilder {
    /// Create a new TLS config builder from server connection settings
    pub fn new(server_conn: &ServerConnection) -> Self {
        Self {
            validate_certs: server_conn.validate_certs,
            ca_cert_path: server_conn.ca_cert_path.clone(),
            insecure: server_conn.insecure,
        }
    }

    /// Build rustls ClientConfig
    pub fn build(&self) -> Result<Arc<ClientConfig>> {
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
        } else if self.validate_certs {
            // Use webpki roots (Mozilla's CA certificates)
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            tracing::debug!("Using webpki system CA certificates for validation");
        }

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Arc::new(config))
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
        let result = builder.build();

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
        let result = builder.build();

        assert!(result.is_ok());
    }
}
