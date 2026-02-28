use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use shared::data::TlsConfig;
use shared::konst::{
    SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE, SHERPA_SERVER_KEY_FILE,
};
use std::path::{Path, PathBuf};

use super::generator::generate_self_signed_certificate;
use super::loader::build_rustls_config;

/// Manages TLS certificates for the Sherpa server
pub struct CertificateManager {
    cert_path: PathBuf,
    key_path: PathBuf,
    config: TlsConfig,
}

impl CertificateManager {
    /// Create a new certificate manager with paths from config
    pub fn new(config: &TlsConfig) -> Result<Self> {
        let cert_path = if let Some(ref path) = config.cert_path {
            PathBuf::from(path)
        } else {
            PathBuf::from(format!(
                "{}/{}/{}",
                SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_CERT_FILE
            ))
        };

        let key_path = if let Some(ref path) = config.key_path {
            PathBuf::from(path)
        } else {
            PathBuf::from(format!(
                "{}/{}/{}",
                SHERPA_BASE_DIR, SHERPA_CERTS_DIR, SHERPA_SERVER_KEY_FILE
            ))
        };

        Ok(Self {
            cert_path,
            key_path,
            config: config.clone(),
        })
    }

    /// Ensure certificates exist (generate if needed and allowed)
    pub async fn ensure_certificates(&self, san: &[String]) -> Result<()> {
        let cert_exists = self.cert_path.exists();
        let key_exists = self.key_path.exists();

        if cert_exists && key_exists {
            tracing::info!("Using existing TLS certificates");
            tracing::info!("  Certificate: {}", self.cert_path.display());
            tracing::info!("  Private key: {}", self.key_path.display());
            return Ok(());
        }

        if !cert_exists || !key_exists {
            if self.config.auto_generate_cert {
                tracing::info!("TLS certificates not found, generating self-signed certificate");
                self.generate_self_signed(san, self.config.cert_validity_days)?;
            } else {
                anyhow::bail!(
                    "TLS certificates not found and auto_generate_cert is disabled.\n  \
                     Expected certificate: {}\n  Expected key: {}",
                    self.cert_path.display(),
                    self.key_path.display()
                );
            }
        }

        Ok(())
    }

    /// Generate a self-signed certificate
    fn generate_self_signed(&self, san: &[String], validity_days: u32) -> Result<()> {
        generate_self_signed_certificate(&self.cert_path, &self.key_path, san, validity_days)
            .context("Failed to generate self-signed certificate")?;

        tracing::info!("Successfully generated self-signed certificate");
        Ok(())
    }

    /// Load certificates and create axum-server RustlsConfig
    pub async fn load_server_config(&self) -> Result<RustlsConfig> {
        tracing::info!("Loading TLS configuration");

        // Validate that both files exist
        if !self.cert_path.exists() {
            anyhow::bail!("Certificate file not found: {}", self.cert_path.display());
        }
        if !self.key_path.exists() {
            anyhow::bail!("Private key file not found: {}", self.key_path.display());
        }

        // Build rustls configuration using axum-server's helper
        let rustls_config = build_rustls_config(&self.cert_path, &self.key_path)
            .await
            .context("Failed to build Rustls configuration")?;

        tracing::info!("Successfully loaded TLS configuration");
        Ok(rustls_config)
    }

    /// Get the certificate file path
    #[allow(dead_code)]
    pub fn cert_path(&self) -> &Path {
        &self.cert_path
    }

    /// Get the private key file path
    #[allow(dead_code)]
    pub fn key_path(&self) -> &Path {
        &self.key_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_certificate_manager_new() {
        let config = TlsConfig {
            enabled: true,
            cert_path: Some("/tmp/test.crt".to_string()),
            key_path: Some("/tmp/test.key".to_string()),
            auto_generate_cert: true,
            cert_validity_days: 365,
            san: vec![],
        };

        let manager = CertificateManager::new(&config).unwrap();
        assert_eq!(manager.cert_path(), Path::new("/tmp/test.crt"));
        assert_eq!(manager.key_path(), Path::new("/tmp/test.key"));
    }

    #[tokio::test]
    async fn test_ensure_certificates_generates_if_missing() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("server.crt");
        let key_path = temp_dir.path().join("server.key");

        let config = TlsConfig {
            enabled: true,
            cert_path: Some(cert_path.to_str().unwrap().to_string()),
            key_path: Some(key_path.to_str().unwrap().to_string()),
            auto_generate_cert: true,
            cert_validity_days: 365,
            san: vec!["IP:127.0.0.1".to_string()],
        };

        let manager = CertificateManager::new(&config).unwrap();
        let result = manager.ensure_certificates(&config.san).await;

        assert!(result.is_ok());
        assert!(cert_path.exists());
        assert!(key_path.exists());
    }
}
