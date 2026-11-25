use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::KeyPair;
use shared::data::{Config, TlsConfig};
use shared::tls::{certificate_scope, compute_key_pin};
use shared::util::write_file_with_permissions;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use tracing::instrument;

use super::generator::write_self_signed_certificate;
use super::loader::{build_rustls_config, load_certificates};
use crate::konst::SHERPA_SERVER_CERT_PATH;
use crate::services::error::ServiceError;

const SHERPA_SERVER_KEY_PATH: &str = "/opt/sherpa/.certs/server.key";
/// An admin's certificate that expires within this many days is warned about.
const EXPIRY_WARNING_DAYS: i64 = 30;

/// Manages TLS certificates for the Sherpa server
pub struct CertificateManager {
    cert_path: PathBuf,
    key_path: PathBuf,
    config: TlsConfig,
}

/// Something wrong with a certificate the admin provides, which sherpad serves anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertificateWarning {
    ExpiresSoon {
        cert_path: PathBuf,
        not_after: jiff::Timestamp,
    },
    AddressesNotCovered {
        cert_path: PathBuf,
        addresses: Vec<IpAddr>,
    },
}

/// What `sherpad status` shows about the server certificate.
#[derive(Debug, Clone)]
pub struct CertificateReport {
    pub cert_path: PathBuf,
    pub key_pin: String,
    pub not_after: jiff::Timestamp,
    pub self_generated: bool,
    pub warnings: Vec<CertificateWarning>,
}

impl fmt::Display for CertificateWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpiresSoon {
                cert_path,
                not_after,
            } => write!(
                f,
                "The server certificate {} expires at {not_after}, within {EXPIRY_WARNING_DAYS} days. Replace it and restart sherpad",
                cert_path.display()
            ),
            Self::AddressesNotCovered {
                cert_path,
                addresses,
            } => {
                let addresses: Vec<String> = addresses.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "The server certificate {} does not name the server's address {}, so clients connecting to it fail the TLS handshake",
                    cert_path.display(),
                    addresses.join(", ")
                )
            }
        }
    }
}

impl CertificateManager {
    /// Create a new certificate manager with paths from config
    pub fn new(config: &TlsConfig) -> Self {
        let cert_path = if let Some(ref path) = config.cert_path {
            PathBuf::from(path)
        } else {
            PathBuf::from(SHERPA_SERVER_CERT_PATH)
        };

        let key_path = if let Some(ref path) = config.key_path {
            PathBuf::from(path)
        } else {
            PathBuf::from(SHERPA_SERVER_KEY_PATH)
        };

        Self {
            cert_path,
            key_path,
            config: config.clone(),
        }
    }

    /// Make the certificate and key ready to serve at startup.
    ///
    /// With `self_generate_cert` on, the key is kept, or made when there is none that loads, and
    /// a new self-signed certificate naming `san` is written. With it off, the admin's files are
    /// checked and never written: a missing one stops startup, and each of
    /// [`CertificateManager::warnings`] is logged, checked against the server's `addresses`.
    ///
    /// # Errors
    ///
    /// Fails when a file sherpad needs cannot be read or written, or, with `self_generate_cert`
    /// off, when the certificate or key is missing.
    #[instrument(skip_all, fields(cert_path = %self.cert_path.display(), key_path = %self.key_path.display()))]
    pub fn prepare(&self, san: &[String], addresses: &[IpAddr]) -> Result<()> {
        if self.config.self_generate_cert {
            let key_pair = self.server_key()?;
            write_self_signed_certificate(
                &self.cert_path,
                &key_pair,
                san,
                self.config.cert_validity_days,
            )
            .context("Failed to generate self-signed certificate")?;
            return Ok(());
        }

        for (kind, path) in [("certificate", &self.cert_path), ("key", &self.key_path)] {
            if !path.exists() {
                anyhow::bail!(ServiceError::TlsFileMissing {
                    kind,
                    path: path.clone(),
                });
            }
        }
        tracing::info!("Serving the admin's TLS certificate and key");
        for warning in self.warnings(addresses, jiff::Timestamp::now())? {
            tracing::warn!("{warning}");
        }
        Ok(())
    }

    /// The server's key: the one in the key file when it loads, or else a new one written there
    /// with mode `0600`.
    fn server_key(&self) -> Result<KeyPair> {
        match fs::read_to_string(&self.key_path) {
            Ok(pem) => match KeyPair::from_pem(&pem) {
                Ok(key_pair) => {
                    tracing::info!(key_path = %self.key_path.display(), "Keeping the server key");
                    return Ok(key_pair);
                }
                Err(error) => tracing::warn!(
                    key_path = %self.key_path.display(),
                    %error,
                    "The server key does not load, making a new one; clients must trust the server again"
                ),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {
                tracing::info!(key_path = %self.key_path.display(), "No server key, making one");
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to read the server key {}", self.key_path.display())
                });
            }
        }
        let key_pair = KeyPair::generate().context("Failed to generate the server key")?;
        write_file_with_permissions(&self.key_path, key_pair.serialize_pem().as_bytes(), 0o600)
            .context("Failed to write the server key")?;
        Ok(key_pair)
    }

    /// What is wrong with the certificate file at `now`: it expires within
    /// `EXPIRY_WARNING_DAYS`, or it does not name every one of the server's `addresses`.
    ///
    /// # Errors
    ///
    /// Fails when the certificate cannot be read or parsed.
    #[instrument(level = "debug", skip_all, fields(cert_path = %self.cert_path.display()))]
    pub fn warnings(
        &self,
        addresses: &[IpAddr],
        now: jiff::Timestamp,
    ) -> Result<Vec<CertificateWarning>> {
        let pem = read_certificate(&self.cert_path)?;
        let scope = certificate_scope(&pem)
            .with_context(|| format!("Failed to parse {}", self.cert_path.display()))?;
        let mut warnings = Vec::new();
        let warn_from = scope
            .not_after
            .checked_sub(jiff::SignedDuration::from_hours(EXPIRY_WARNING_DAYS * 24))
            .context("The certificate's expiry is out of range")?;
        if now >= warn_from {
            warnings.push(CertificateWarning::ExpiresSoon {
                cert_path: self.cert_path.clone(),
                not_after: scope.not_after,
            });
        }
        let uncovered: Vec<IpAddr> = addresses
            .iter()
            .filter(|address| !scope.ip_addresses.contains(address))
            .copied()
            .collect();
        if !uncovered.is_empty() {
            warnings.push(CertificateWarning::AddressesNotCovered {
                cert_path: self.cert_path.clone(),
                addresses: uncovered,
            });
        }
        Ok(warnings)
    }

    /// The certificate's key pin and expiry, and, with `self_generate_cert` off, what is wrong
    /// with it at `now` for the server's `addresses`.
    ///
    /// # Errors
    ///
    /// Fails when the certificate cannot be read or parsed.
    #[instrument(level = "debug", skip_all, fields(cert_path = %self.cert_path.display()))]
    pub fn report(&self, addresses: &[IpAddr], now: jiff::Timestamp) -> Result<CertificateReport> {
        let pem = read_certificate(&self.cert_path)?;
        let scope = certificate_scope(&pem)
            .with_context(|| format!("Failed to parse {}", self.cert_path.display()))?;
        let warnings = if self.config.self_generate_cert {
            Vec::new()
        } else {
            self.warnings(addresses, now)?
        };
        Ok(CertificateReport {
            cert_path: self.cert_path.clone(),
            key_pin: self.key_pin()?,
            not_after: scope.not_after,
            self_generated: self.config.self_generate_cert,
            warnings,
        })
    }

    /// Load certificates and create axum-server `RustlsConfig`
    #[instrument(skip_all, fields(cert_path = %self.cert_path.display()))]
    pub async fn load_server_config(&self) -> Result<RustlsConfig> {
        tracing::info!("Loading TLS configuration");

        let rustls_config = build_rustls_config(&self.cert_path, &self.key_path)
            .await
            .context("Failed to build Rustls configuration")?;

        tracing::info!("Successfully loaded TLS configuration");
        tracing::info!(
            key_pin = %self.key_pin()?,
            "TLS key pin (SHA-256 of the server's public key), which clients ask users to check on first connect"
        );
        Ok(rustls_config)
    }

    /// The key pin of the server certificate's public key, as `sherpa` shows it on first
    /// connect.
    ///
    /// # Errors
    ///
    /// Fails when the certificate cannot be read or parsed.
    pub fn key_pin(&self) -> Result<String> {
        let certificates = load_certificates(&self.cert_path)?;
        let certificate = certificates
            .first()
            .with_context(|| format!("No certificate in {}", self.cert_path.display()))?;
        compute_key_pin(certificate)
    }
}

/// The addresses the server listens on that a certificate must name: `server_ipv4` and
/// `server_ipv6`, each unless it is the unspecified address.
#[must_use]
pub fn server_addresses(config: &Config) -> Vec<IpAddr> {
    let ipv4 = Some(IpAddr::V4(config.server_ipv4));
    let ipv6 = config.server_ipv6.map(IpAddr::V6);
    [ipv4, ipv6]
        .into_iter()
        .flatten()
        .filter(|address| !address.is_unspecified())
        .collect()
}

/// The text of the certificate file at `path`.
fn read_certificate(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("Failed to read the certificate {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_capture::capture_log;
    use shared::tls::extract_cert_info;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    const SERVER_ADDRESS: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 10));

    /// A manager for files in `dir`, with `self_generate_cert` set to `self_generate`.
    fn manager_in(dir: &TempDir, self_generate: bool) -> CertificateManager {
        CertificateManager::new(&TlsConfig {
            cert_path: Some(dir.path().join("server.crt").display().to_string()),
            key_path: Some(dir.path().join("server.key").display().to_string()),
            self_generate_cert: self_generate,
            ..TlsConfig::default()
        })
    }

    fn san_of(address: &str) -> Vec<String> {
        vec![format!("IP:{address}")]
    }

    /// Write a certificate the admin made, for `address`, valid for `days`, with its key.
    fn write_admin_certificate(manager: &CertificateManager, address: IpAddr, days: u32) {
        let key_pair = KeyPair::generate().unwrap();
        fs::write(&manager.key_path, key_pair.serialize_pem()).unwrap();
        write_self_signed_certificate(
            &manager.cert_path,
            &key_pair,
            &san_of(&address.to_string()),
            days,
        )
        .unwrap();
    }

    #[test]
    fn server_certificate_001_a_start_with_no_certificate_writes_it_with_a_0600_key() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);

        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        assert!(manager.cert_path.exists());
        let mode = fs::metadata(&manager.key_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn server_certificate_003_the_logged_key_pin_is_the_one_clients_are_asked_to_check() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        manager.prepare(&san_of("127.0.0.1"), &[]).unwrap();

        let pem = fs::read_to_string(&manager.cert_path).unwrap();
        let shown = extract_cert_info("wss://127.0.0.1:3030/ws", &pem).unwrap();
        assert_eq!(manager.key_pin().unwrap(), shown.key_pin);
    }

    #[test]
    fn server_certificate_005_with_self_generate_cert_off_a_missing_file_stops_startup_and_names_itself()
     {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, false);
        write_admin_certificate(&manager, SERVER_ADDRESS, 365);
        fs::remove_file(&manager.key_path).unwrap();

        let error = manager.prepare(&[], &[SERVER_ADDRESS]).unwrap_err();

        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::TlsFileMissing {
                kind: "key",
                path: manager.key_path.clone(),
            })
        );
    }

    #[test]
    fn server_certificate_009_each_start_writes_a_new_certificate_with_the_same_key() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();
        let first_cert = fs::read_to_string(&manager.cert_path).unwrap();
        let first_key = fs::read_to_string(&manager.key_path).unwrap();
        let first_pin = manager.key_pin().unwrap();

        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        assert_ne!(fs::read_to_string(&manager.cert_path).unwrap(), first_cert);
        assert_eq!(fs::read_to_string(&manager.key_path).unwrap(), first_key);
        assert_eq!(manager.key_pin().unwrap(), first_pin);
    }

    #[test]
    fn server_certificate_017_each_certificate_made_for_the_kept_key_has_its_own_serial_number() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        let serial = || {
            let der = &load_certificates(&manager.cert_path).unwrap()[0];
            let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
            cert.raw_serial().to_vec()
        };
        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();
        let first = serial();

        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        assert_ne!(serial(), first);
    }

    #[test]
    fn server_certificate_010_a_start_with_no_key_makes_a_new_one() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();
        let first_pin = manager.key_pin().unwrap();
        fs::remove_file(&manager.key_path).unwrap();

        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        assert!(manager.key_path.exists());
        assert_ne!(manager.key_pin().unwrap(), first_pin);
    }

    #[test]
    fn server_certificate_011_a_key_file_that_does_not_load_is_replaced_by_a_new_key() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        fs::write(&manager.key_path, "not a key").unwrap();

        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        let key = fs::read_to_string(&manager.key_path).unwrap();
        KeyPair::from_pem(&key).unwrap();
        let mode = fs::metadata(&manager.key_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn server_certificate_012_a_start_after_the_address_changed_names_the_new_address() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, true);
        manager.prepare(&san_of("192.0.2.10"), &[]).unwrap();

        manager.prepare(&san_of("192.0.2.20"), &[]).unwrap();

        let pem = fs::read_to_string(&manager.cert_path).unwrap();
        let addresses = certificate_scope(&pem).unwrap().ip_addresses;
        assert!(
            addresses.contains(&"192.0.2.20".parse().unwrap()),
            "{addresses:?}"
        );
        assert!(!addresses.contains(&SERVER_ADDRESS), "{addresses:?}");
    }

    #[test]
    fn server_certificate_013_with_self_generate_cert_off_a_start_writes_neither_file() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, false);
        write_admin_certificate(&manager, SERVER_ADDRESS, 365);
        let cert = fs::read(&manager.cert_path).unwrap();
        let key = fs::read(&manager.key_path).unwrap();

        manager
            .prepare(&san_of("192.0.2.20"), &[SERVER_ADDRESS])
            .unwrap();

        assert_eq!(fs::read(&manager.cert_path).unwrap(), cert);
        assert_eq!(fs::read(&manager.key_path).unwrap(), key);
    }

    #[test]
    fn server_certificate_014_an_admin_certificate_expiring_within_30_days_is_warned_about_at_start()
     {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, false);
        write_admin_certificate(&manager, SERVER_ADDRESS, 29);

        let (result, output) = capture_log(|| manager.prepare(&[], &[SERVER_ADDRESS]));

        result.unwrap();
        assert!(output.contains("WARN"), "{output}");
        assert!(output.contains("within 30 days"), "{output}");
    }

    #[test]
    fn server_certificate_015_an_admin_certificate_not_naming_the_server_address_is_warned_about_at_start()
     {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, false);
        write_admin_certificate(&manager, "192.0.2.20".parse().unwrap(), 365);

        let (result, output) = capture_log(|| manager.prepare(&[], &[SERVER_ADDRESS]));

        result.unwrap();
        assert!(output.contains("WARN"), "{output}");
        assert!(
            output.contains("does not name the server's address 192.0.2.10"),
            "{output}"
        );
        assert!(!output.contains("within 30 days"), "{output}");
    }

    #[test]
    fn server_certificate_016_the_status_report_carries_the_admin_certificates_warnings() {
        let dir = TempDir::new().unwrap();
        let manager = manager_in(&dir, false);
        write_admin_certificate(&manager, "192.0.2.20".parse().unwrap(), 10);

        let report = manager
            .report(&[SERVER_ADDRESS], jiff::Timestamp::now())
            .unwrap();

        assert!(matches!(
            report.warnings.as_slice(),
            [
                CertificateWarning::ExpiresSoon { .. },
                CertificateWarning::AddressesNotCovered { .. }
            ]
        ));
    }
}
