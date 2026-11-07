use anyhow::{Context, Result};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConfig, RootCertStore};
use std::io::{self, IsTerminal};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::data::ServerConnection;
use crate::tls::cert_fetch::fetch_server_certificate;
use crate::tls::trust_store::{
    CertificateInfo, TrustStore, extract_cert_info, parse_certificate_pem,
};
use crate::util::{Emoji, confirm, term_msg_surround, term_msg_underline};

/// Builds TLS configuration for WebSocket client
pub struct TlsConfigBuilder {
    validate_certs: bool,
    ca_cert_path: Option<String>,
    insecure: bool,
    http_port: u16,
}

/// The TLS configuration to connect with. When it trusts only public CAs, it also records
/// whether the handshake refused the server's certificate for having no public issuer, so the
/// caller can fall back to trusting it on first use.
pub struct ServerTls {
    pub config: Arc<ClientConfig>,
    unknown_issuer: Option<Arc<AtomicBool>>,
}

impl ServerTls {
    /// Whether the handshake refused the server's certificate because no public CA issued it,
    /// as with Sherpa's own self-signed certificate.
    #[must_use]
    pub fn refused_unknown_issuer(&self) -> bool {
        self.unknown_issuer
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

impl TlsConfigBuilder {
    /// Create a new TLS config builder from server connection settings
    #[must_use]
    pub fn new(server_conn: &ServerConnection) -> Self {
        Self {
            validate_certs: server_conn.validate_certs,
            ca_cert_path: server_conn.ca_cert_path.clone(),
            insecure: server_conn.insecure,
            http_port: server_conn.http_port,
        }
    }

    /// Build the TLS configuration for `server_url`:
    /// 1. Insecure mode skips validation; a custom CA is the only one trusted.
    /// 2. A certificate already in the trust store is trusted, along with the public CAs when
    ///    `validate_certs` is on.
    /// 3. Otherwise, with `validate_certs` on, only the public CAs are trusted; if the handshake
    ///    then refuses the certificate (`ServerTls::refused_unknown_issuer`), the caller runs
    ///    `trust_on_first_use`.
    /// 4. With `validate_certs` off, the user is asked to trust the server's certificate now.
    ///
    /// # Errors
    ///
    /// Fails when the custom CA or a trusted certificate cannot be loaded, the trust store cannot be opened, or, with `validate_certs` off, the user does not trust the server's certificate.
    pub async fn build_with_trust_flow(&self, server_url: &str) -> Result<ServerTls> {
        if self.insecure {
            tracing::warn!("INSECURE MODE: TLS certificate validation is DISABLED");
            return Ok(ServerTls {
                config: Arc::new(Self::build_insecure()),
                unknown_issuer: None,
            });
        }

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
            return Ok(ServerTls {
                config: Arc::new(config),
                unknown_issuer: None,
            });
        }

        let trust_store =
            TrustStore::new().context("Failed to initialize certificate trust store")?;
        if let Some(cert_pem) = trust_store.get_cert(server_url)? {
            tracing::info!("Using trusted certificate for: {}", server_url);
            return Ok(ServerTls {
                config: self.trusted_config(&cert_pem)?,
                unknown_issuer: None,
            });
        }

        if self.validate_certs {
            tracing::debug!("Trying the public CAs before trusting on first use");
            let unknown_issuer = Arc::new(AtomicBool::new(false));
            let verifier = PublicCaVerifier::new(Arc::clone(&unknown_issuer))?;
            let config = ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            return Ok(ServerTls {
                config: Arc::new(config),
                unknown_issuer: Some(unknown_issuer),
            });
        }

        Ok(ServerTls {
            config: self.trust_on_first_use(server_url).await?,
            unknown_issuer: None,
        })
    }

    /// Fetch the server's certificate, show it and ask the user to trust it; a trusted
    /// certificate is saved to the trust store and the configuration trusting it is returned.
    ///
    /// # Errors
    ///
    /// Fails when the certificate cannot be fetched or parsed, the user does not trust it, there is no terminal to ask on, or it cannot be saved.
    pub async fn trust_on_first_use(&self, server_url: &str) -> Result<Arc<ClientConfig>> {
        tracing::info!("Certificate not found in trust store, fetching from server...");
        let fetched_cert = fetch_server_certificate(server_url, self.http_port)
            .await
            .context("Failed to fetch server certificate")?;

        let cert_info = extract_cert_info(server_url, &fetched_cert)
            .context("Failed to parse certificate information")?;

        println!();
        print_certificate_info(&cert_info);
        println!(
            "{} WARNING: This is the first time connecting to this server.",
            Emoji::Warning
        );
        println!("   Verify the fingerprint matches what you expect to prevent");
        println!("   man-in-the-middle attacks.");
        println!();

        if !prompt_trust_certificate()? {
            anyhow::bail!(
                "Certificate not trusted by user.\n\n\
                 Connection aborted. To bypass certificate validation (not recommended),\n\
                 use the --insecure flag."
            );
        }

        let trust_store =
            TrustStore::new().context("Failed to initialize certificate trust store")?;
        trust_store
            .save_cert(server_url, &fetched_cert)
            .context("Failed to save certificate to trust store")?;

        println!("{} Certificate saved to trust store", Emoji::Success);
        println!();

        self.trusted_config(&fetched_cert)
    }

    /// A configuration trusting `cert_pem`, and the public CAs when `validate_certs` is on.
    fn trusted_config(&self, cert_pem: &str) -> Result<Arc<ClientConfig>> {
        let mut root_store = RootCertStore::empty();
        let cert =
            parse_certificate_pem(cert_pem).context("Failed to parse trusted certificate")?;
        root_store
            .add(cert)
            .context("Failed to add trusted certificate to root store")?;
        if self.validate_certs {
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Arc::new(config))
    }

    /// Load a CA certificate from file
    fn load_ca_cert(path: &Path) -> Result<CertificateDer<'static>> {
        let pem = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to open CA certificate file: {}", path.display()))?;
        parse_certificate_pem(&pem)
            .with_context(|| format!("No certificate in CA file: {}", path.display()))
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

/// Verifies the server's certificate against the public CAs (webpki roots), recording when it
/// is refused for having no public issuer, so the caller can fall back to trust on first use.
#[derive(Debug)]
struct PublicCaVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    unknown_issuer: Arc<AtomicBool>,
}

impl PublicCaVerifier {
    fn new(unknown_issuer: Arc<AtomicBool>) -> Result<Self> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let webpki = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .build()
        .context("Failed to build the public CA verifier")?;
        Ok(Self {
            webpki,
            unknown_issuer,
        })
    }
}

impl ServerCertVerifier for PublicCaVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self.webpki.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        if let Err(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)) = verified {
            self.unknown_issuer.store(true, Ordering::Relaxed);
        }
        verified
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

/// Custom certificate verifier that accepts anything (INSECURE - for development only)
#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Accept any certificate without validation
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    /// Every scheme the crypto provider verifies, so any server key works, RSA-PSS included,
    /// which TLS 1.3 requires for RSA.
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Show a certificate's server, subject, issuer, validity and SHA-256 fingerprint.
pub fn print_certificate_info(cert_info: &CertificateInfo) {
    term_msg_surround("Certificate Information");
    println!();
    println!("  Server:      {}", cert_info.server_url);
    println!("  Subject:     {}", cert_info.subject);
    if cert_info.self_signed {
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
}

/// Ask the user whether to trust a server's certificate on first connection.
fn prompt_trust_certificate() -> Result<bool> {
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
    confirm("Do you trust this certificate?")
}

/// Check if running in an interactive terminal
fn is_interactive() -> bool {
    io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insecure_mode_accepts_every_scheme_the_provider_verifies() {
        let schemes = NoCertificateVerification.supported_verify_schemes();
        assert!(schemes.contains(&rustls::SignatureScheme::RSA_PSS_SHA256));
        assert!(schemes.contains(&rustls::SignatureScheme::ECDSA_NISTP384_SHA384));
    }

    const SELF_SIGNED: &str = include_str!("testdata/self_signed.crt");

    #[test]
    fn a_certificate_no_public_ca_issued_is_refused_and_flagged() {
        let unknown_issuer = Arc::new(AtomicBool::new(false));
        let verifier = PublicCaVerifier::new(Arc::clone(&unknown_issuer)).unwrap();
        let cert = parse_certificate_pem(SELF_SIGNED).unwrap();
        let name = ServerName::try_from("localhost").unwrap();

        let outcome =
            verifier.verify_server_cert(&cert, &[], &name, &[], rustls::pki_types::UnixTime::now());

        assert!(outcome.is_err());
        assert!(unknown_issuer.load(Ordering::Relaxed), "{outcome:?}");
    }

    #[tokio::test]
    async fn insecure_mode_never_falls_back_to_trust_on_first_use() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .ok();
        let server_conn = ServerConnection {
            insecure: true,
            ..ServerConnection::default()
        };
        let tls = TlsConfigBuilder::new(&server_conn)
            .build_with_trust_flow("wss://localhost:3030/ws")
            .await
            .unwrap();
        assert!(!tls.refused_unknown_issuer());
    }

    #[tokio::test]
    async fn a_custom_ca_is_trusted_without_the_trust_store() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .ok();
        let ca = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ca.path(), SELF_SIGNED).unwrap();
        let server_conn = ServerConnection {
            ca_cert_path: Some(ca.path().display().to_string()),
            ..ServerConnection::default()
        };
        let tls = TlsConfigBuilder::new(&server_conn)
            .build_with_trust_flow("wss://localhost:3030/ws")
            .await
            .unwrap();
        assert!(!tls.refused_unknown_issuer());
    }
}
