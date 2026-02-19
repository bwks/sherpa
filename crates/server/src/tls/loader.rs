use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Load certificates from PEM file
#[allow(dead_code)]
pub fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open certificate file: {}", path.display()))?;
    let mut reader = BufReader::new(file);

    let certs = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificates from PEM file")?;

    if certs.is_empty() {
        anyhow::bail!("No certificates found in file: {}", path.display());
    }

    tracing::debug!(
        "Loaded {} certificate(s) from {}",
        certs.len(),
        path.display()
    );
    Ok(certs)
}

/// Load private key from PEM file
#[allow(dead_code)]
pub fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open private key file: {}", path.display()))?;
    let mut reader = BufReader::new(file);

    let key = private_key(&mut reader)
        .context("Failed to parse private key from PEM file")?
        .ok_or_else(|| anyhow::anyhow!("No private key found in file: {}", path.display()))?;

    tracing::debug!("Loaded private key from {}", path.display());
    Ok(key)
}

/// Build axum-server RustlsConfig from certificate and key paths
pub async fn build_rustls_config(cert_path: &Path, key_path: &Path) -> Result<RustlsConfig> {
    let config = RustlsConfig::from_pem_file(cert_path, key_path)
        .await
        .context("Failed to build Rustls configuration from PEM files")?;

    tracing::debug!("Built axum-server Rustls configuration");
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Sample self-signed certificate for testing
    const TEST_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDazCCAlOgAwIBAgIUfPXqR/f7V8OBLaHnfHXLqNr7qnEwDQYJKoZIhvcNAQEL
BQAwRTELMAkGA1UEBhMCVVMxEzARBgNVBAgMClNvbWUtU3RhdGUxITAfBgNVBAoM
GEludGVybmV0IFdpZGdpdHMgUHR5IEx0ZDAeFw0yNDAxMDEwMDAwMDBaFw0yNTAx
MDEwMDAwMDBaMEUxCzAJBgNVBAYTAlVTMRMwEQYDVQQIDApTb21lLVN0YXRlMSEw
HwYDVQQKDBhJbnRlcm5ldCBXaWRnaXRzIFB0eSBMdGQwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQC7VJTUt9Us8cKjMzEfYyjiWA4/qMD/Cw5YayVL76Vf
eWgjpWLPUvvF2z2UNvqL+YmZzPKNxFpM9HH0Sg+qvUTtaPPXYDq8xXNPqfmKwvAv
9nT3JYsKN4OQaJw4I3mCfhNq7nFjv0SkPCPmWk6JTdL8UaqdWDKFuJqPEuTt3ZMk
8XT9bGX7w3vxGiX0eWdCDpKQb8R8lL8v3JePdKjkXnNKKbvNrJ1BsUqJYQCT6Rlb
bCvEGW0aEm0jcqsOz0l3y5cS0OQi3L7GbNMJPdGtbEJfMPKpEqZ5U/JqPjz6cIqP
7bKT0KWxQDhDCQTvMd8lZxOCpKWjE4j2MkVMqLTqQqZbAgMBAAGjUzBRMB0GA1Ud
DgQWBBRrHe8Xy4GUQdKEWJzMqqSLLF2lawAfBgNVHSMEGDAWgBRrHe8Xy4GUQdKE
WJzMqqSLLF2lawAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQCb
NNBmU7WMQWT3MqHrHqUw3qSPRXhP3RGPfZjJGk7VTLJHpq8mAJrBrqH0XVXQLJcK
tP3xXKhLPeZPCPjqRNQRBZ5UpHhRQxKxXD5p8PxKCOkKq3BnF0yNJZr6LVGPqDxF
cQTjHvXJPLqHrJvXLbqf2CmFZJVhSRU0vUqQ3eG4KMTdnYCQ3FhPvBJYZLJyKbf3
2lxE3J8Vk7wLLqCE4lLFVjwHxKkKqMqvXQPJxCYQWwxQUWvVcGF4SxLVNJNqVlxy
t3w8G6kVOQdMdYKqbQT8qWNuEJWvDzKLZlJxXvhR0YJn3Y7DRXPqKQxzVjCVJYqZ
LkK3HQdFGQWqVGqfyN5x
-----END CERTIFICATE-----"#;

    const TEST_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC7VJTUt9Us8cKj
MzEfYyjiWA4/qMD/Cw5YayVL76VfeWgjpWLPUvvF2z2UNvqL+YmZzPKNxFpM9HH0
Sg+qvUTtaPPXYDq8xXNPqfmKwvAv9nT3JYsKN4OQaJw4I3mCfhNq7nFjv0SkPCPm
Wk6JTdL8UaqdWDKFuJqPEuTt3ZMk8XT9bGX7w3vxGiX0eWdCDpKQb8R8lL8v3JeP
dKjkXnNKKbvNrJ1BsUqJYQCT6RlbbCvEGW0aEm0jcqsOz0l3y5cS0OQi3L7GbNMJ
PdGtbEJfMPKpEqZ5U/JqPjz6cIqP7bKT0KWxQDhDCQTvMd8lZxOCpKWjE4j2MkVM
qLTqQqZbAgMBAAECggEAFQE7f3CHY3K0qVlLF9QoqP8gNW8EHJvLvGz3GDPmQ1Kj
p0m6V4FPxPqR8cHQAQnqJqkqF2aDdPPPcGvB+QJ0vXCLHnD5vQtPvHvPvXJN8S5p
TvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqT
vWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGv
T5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVq
T8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQQ
KBgQDmGvN8fKGQdNvxE3vPvXJN8S5pTvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqG
vT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvV
qT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQ
QKBgQDRvN8fKGQdNvxE3vPvXJN8S5pTvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqG
vT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvV
qT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQ
QKBgCvN8fKGQdNvxE3vPvXJN8S5pTvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqGvT
5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT
8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQQK
BgQCvN8fKGQdNvxE3vPvXJN8S5pTvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqGvT5
vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8
JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQQKB
gBvN8fKGQdNvxE3vPvXJN8S5pTvEQbvDqGvFvFxVqTvWqQVwvXvVqT8JqGvT5vV
qTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8Jq
GvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQVwvXvVqT8JqGvT5vVqTvWqQ
-----END PRIVATE KEY-----"#;

    #[test]
    fn test_load_certificates() {
        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(TEST_CERT_PEM.as_bytes()).unwrap();
        cert_file.flush().unwrap();

        let result = load_certificates(cert_file.path());
        assert!(result.is_ok());

        let certs = result.unwrap();
        assert!(!certs.is_empty());
    }

    #[test]
    fn test_load_private_key() {
        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(TEST_KEY_PEM.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let result = load_private_key(key_file.path());
        assert!(result.is_ok());
    }
}
