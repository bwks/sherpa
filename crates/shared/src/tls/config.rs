use anyhow::{Context, Result};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs;
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConfig, RootCertStore};
use std::io::{self, IsTerminal};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::data::ServerConnection;
use crate::tls::cert_fetch::fetch_server_certificate;
use crate::tls::trust_store::{
    CertificateInfo, TrustStore, compute_key_pin, extract_cert_info, parse_certificate_pem,
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
    /// 2. A server whose public key is the key of the certificate saved for it in the trust store
    ///    is trusted, whatever certificate it now presents for that key; otherwise, when
    ///    `validate_certs` is on, a certificate a public CA issued is.
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
        println!("   Verify the key pin matches what you expect to prevent");
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

    /// A configuration trusting a server that presents the public key of `cert_pem`, and the
    /// public CAs when `validate_certs` is on.
    fn trusted_config(&self, cert_pem: &str) -> Result<Arc<ClientConfig>> {
        let verifier = PinnedKeyVerifier::new(cert_pem, self.validate_certs)?;
        let config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
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

/// Trusts a server whose certificate carries the public key the user trusted (its key pin),
/// whatever its names and dates, so a certificate the server made again with the same key needs
/// no new question. Any other certificate is checked against the public CAs when `public_cas`
/// is set, and refused otherwise.
#[derive(Debug)]
struct PinnedKeyVerifier {
    key_pin: String,
    public_cas: Option<Arc<WebPkiServerVerifier>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedKeyVerifier {
    /// A verifier pinning the public key of `cert_pem`, falling back to the public CAs when
    /// `validate_certs` is on.
    fn new(cert_pem: &str, validate_certs: bool) -> Result<Self> {
        let cert =
            parse_certificate_pem(cert_pem).context("Failed to parse trusted certificate")?;
        let key_pin = compute_key_pin(&cert)?;
        let public_cas = if validate_certs {
            Some(public_ca_verifier()?)
        } else {
            None
        };
        Ok(Self {
            key_pin,
            public_cas,
            algorithms: aws_lc_rs::default_provider().signature_verification_algorithms,
        })
    }
}

impl ServerCertVerifier for PinnedKeyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if compute_key_pin(end_entity).is_ok_and(|pin| pin == self.key_pin) {
            return Ok(ServerCertVerified::assertion());
        }
        match &self.public_cas {
            Some(public_cas) => public_cas.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            None => Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A verifier of certificates against the public CAs (webpki roots).
fn public_ca_verifier() -> Result<Arc<WebPkiServerVerifier>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(aws_lc_rs::default_provider()),
    )
    .build()
    .context("Failed to build the public CA verifier")
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
        Ok(Self {
            webpki: public_ca_verifier()?,
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
        aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Show a certificate's server, subject, issuer, validity and the key pin of its public key.
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
    term_msg_underline("Key Pin (SHA-256 of the server's public key)");
    println!("  {}", cert_info.key_pin);
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
    fn certificate_trust_006_insecure_accepts_each_signature_scheme_the_library_can_check() {
        let schemes = NoCertificateVerification.supported_verify_schemes();
        assert!(schemes.contains(&rustls::SignatureScheme::RSA_PSS_SHA256));
        assert!(schemes.contains(&rustls::SignatureScheme::ECDSA_NISTP384_SHA384));
    }

    const SELF_SIGNED: &str = include_str!("testdata/self_signed.crt");

    #[test]
    fn certificate_trust_003_a_certificate_no_public_ca_issued_is_refused_and_flagged() {
        let unknown_issuer = Arc::new(AtomicBool::new(false));
        let verifier = PublicCaVerifier::new(Arc::clone(&unknown_issuer)).unwrap();
        let cert = parse_certificate_pem(SELF_SIGNED).unwrap();
        let name = ServerName::try_from("localhost").unwrap();

        let outcome =
            verifier.verify_server_cert(&cert, &[], &name, &[], rustls::pki_types::UnixTime::now());

        assert!(outcome.is_err());
        assert!(unknown_issuer.load(Ordering::Relaxed), "{outcome:?}");
    }

    /// Two certificates the server made with one key, as on two starts, and one with another key.
    const FIRST_START: &str = include_str!("testdata/same_key_first.crt");
    const SECOND_START: &str = include_str!("testdata/same_key_second.crt");

    /// Whether a client that trusted `trusted` connects to a server presenting `presented`.
    fn connects(trusted: &str, presented: &str) -> bool {
        let verifier = PinnedKeyVerifier::new(trusted, true).unwrap();
        let cert = parse_certificate_pem(presented).unwrap();
        let name = ServerName::try_from("192.0.2.10").unwrap();
        verifier
            .verify_server_cert(&cert, &[], &name, &[], rustls::pki_types::UnixTime::now())
            .is_ok()
    }

    #[test]
    fn certificate_trust_018_a_server_presenting_another_key_than_the_one_trusted_is_refused() {
        assert!(!connects(FIRST_START, SELF_SIGNED));
    }

    #[test]
    fn certificate_trust_019_a_certificate_made_again_with_the_trusted_key_is_trusted_without_a_question()
     {
        assert_ne!(FIRST_START, SECOND_START);
        assert!(connects(FIRST_START, SECOND_START));
    }

    #[tokio::test]
    async fn certificate_trust_005_insecure_connects_without_asking_fetching_or_saving() {
        aws_lc_rs::default_provider().install_default().ok();
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
    async fn certificate_trust_007_a_certificate_from_the_configured_ca_is_trusted_without_a_question()
     {
        aws_lc_rs::default_provider().install_default().ok();
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
