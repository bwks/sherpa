use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Holds the generated lab CA certificate and key pair for signing node certificates
pub struct LabCa {
    pub cert: Certificate,
    pub key_pair: KeyPair,
}

/// Generate a self-signed certificate with the given parameters
pub fn generate_self_signed_certificate(
    cert_path: &Path,
    key_path: &Path,
    san: &[String],
    validity_days: u32,
) -> Result<()> {
    tracing::info!("Generating self-signed TLS certificate");

    // Generate key pair first
    let key_pair = KeyPair::generate().context("Failed to generate key pair")?;

    // Create certificate parameters
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create certificate parameters")?;

    // Set distinguished name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Sherpa Server");
    dn.push(DnType::OrganizationName, "Sherpa");
    params.distinguished_name = dn;

    // Add Subject Alternative Names
    for san_entry in san {
        if let Some(ip_str) = san_entry.strip_prefix("IP:") {
            if let Ok(ip) = ip_str.parse() {
                params.subject_alt_names.push(SanType::IpAddress(ip));
                tracing::debug!("Added IP SAN: {}", ip_str);
            } else {
                tracing::warn!("Invalid IP in SAN: {}", ip_str);
            }
        } else if let Some(dns) = san_entry.strip_prefix("DNS:") {
            params.subject_alt_names.push(SanType::DnsName(
                dns.to_string()
                    .try_into()
                    .context(format!("Invalid DNS SAN: {}", dns))?,
            ));
            tracing::debug!("Added DNS SAN: {}", dns);
        } else {
            // Assume it's a DNS name if no prefix
            params.subject_alt_names.push(SanType::DnsName(
                san_entry
                    .to_string()
                    .try_into()
                    .context(format!("Invalid DNS SAN: {}", san_entry))?,
            ));
            tracing::debug!("Added DNS SAN: {}", san_entry);
        }
    }

    // Add localhost and 127.0.0.1 if not already present
    let has_localhost = san.iter().any(|s| s.contains("localhost"));
    let has_loopback = san.iter().any(|s| s.contains("127.0.0.1"));

    if !has_localhost {
        params.subject_alt_names.push(SanType::DnsName(
            "localhost"
                .to_string()
                .try_into()
                .context("Failed to create localhost SAN")?,
        ));
        tracing::debug!("Added default DNS SAN: localhost");
    }

    if !has_loopback {
        params.subject_alt_names.push(SanType::IpAddress(
            "127.0.0.1"
                .parse()
                .context("Failed to parse loopback address")?,
        ));
        tracing::debug!("Added default IP SAN: 127.0.0.1");
    }

    // Set validity period using jiff for time calculation
    // Note: Timestamp only supports units of hours or smaller, so convert days to hours
    let now = jiff::Timestamp::now();
    let validity_hours = (validity_days as i64) * 24;
    let future = now
        .checked_add(jiff::Span::new().hours(validity_hours))
        .context("Failed to calculate certificate expiration date")?;

    // Convert jiff timestamps to time::OffsetDateTime for rcgen
    // rcgen requires time::OffsetDateTime, so we convert via Unix timestamp
    let not_before = time::OffsetDateTime::from_unix_timestamp(now.as_second())
        .context("Failed to convert start time to OffsetDateTime")?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(future.as_second())
        .context("Failed to convert end time to OffsetDateTime")?;

    params.not_before = not_before;
    params.not_after = not_after;

    // Generate certificate
    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    // Write certificate to file
    let cert_pem = cert.pem();
    write_file_with_permissions(cert_path, cert_pem.as_bytes(), 0o644)
        .context("Failed to write certificate file")?;

    tracing::info!("Certificate written to: {}", cert_path.display());

    // Write private key to file
    let key_pem = key_pair.serialize_pem();
    write_file_with_permissions(key_path, key_pem.as_bytes(), 0o600)
        .context("Failed to write private key file")?;

    tracing::info!("Private key written to: {}", key_path.display());
    tracing::info!("Certificate valid for {} days", validity_days);

    Ok(())
}

/// Generate a CA certificate for a lab. Returns the CA cert and key pair
/// for use when signing node certificates.
pub fn generate_lab_ca(
    cert_path: &Path,
    key_path: &Path,
    lab_id: &str,
    validity_days: u32,
) -> Result<LabCa> {
    tracing::info!(lab_id = %lab_id, "Generating lab CA certificate");

    let key_pair = KeyPair::generate().context("Failed to generate CA key pair")?;

    let mut params =
        CertificateParams::new(vec![]).context("Failed to create CA certificate parameters")?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("Sherpa Lab CA - {}", lab_id));
    dn.push(DnType::OrganizationName, "Sherpa");
    params.distinguished_name = dn;

    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    let now = jiff::Timestamp::now();
    let validity_hours = (validity_days as i64) * 24;
    let future = now
        .checked_add(jiff::Span::new().hours(validity_hours))
        .context("Failed to calculate CA certificate expiration date")?;

    let not_before = time::OffsetDateTime::from_unix_timestamp(now.as_second())
        .context("Failed to convert start time to OffsetDateTime")?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(future.as_second())
        .context("Failed to convert end time to OffsetDateTime")?;

    params.not_before = not_before;
    params.not_after = not_after;

    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate lab CA certificate")?;

    let cert_pem = cert.pem();
    write_file_with_permissions(cert_path, cert_pem.as_bytes(), 0o644)
        .context("Failed to write CA certificate file")?;

    let key_pem = key_pair.serialize_pem();
    write_file_with_permissions(key_path, key_pem.as_bytes(), 0o600)
        .context("Failed to write CA private key file")?;

    tracing::info!(
        lab_id = %lab_id,
        "Lab CA certificate generated: {}",
        cert_path.display()
    );

    Ok(LabCa { cert, key_pair })
}

/// Generate a node certificate signed by the lab CA
pub fn generate_node_certificate(
    cert_path: &Path,
    key_path: &Path,
    lab_ca: &LabCa,
    hostname: &str,
    ip: &str,
    validity_days: u32,
) -> Result<()> {
    tracing::info!(hostname = %hostname, "Generating node certificate");

    let node_key_pair = KeyPair::generate().context("Failed to generate node key pair")?;

    let mut params =
        CertificateParams::new(vec![]).context("Failed to create node certificate parameters")?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname);
    dn.push(DnType::OrganizationName, "Sherpa");
    params.distinguished_name = dn;

    // Add SANs
    params.subject_alt_names.push(SanType::DnsName(
        hostname
            .to_string()
            .try_into()
            .context("Failed to convert hostname to DNS name for SAN")?,
    ));

    if let Ok(ip_addr) = ip.parse() {
        params.subject_alt_names.push(SanType::IpAddress(ip_addr));
    }

    let now = jiff::Timestamp::now();
    let validity_hours = (validity_days as i64) * 24;
    let future = now
        .checked_add(jiff::Span::new().hours(validity_hours))
        .context("Failed to calculate node certificate expiration date")?;

    let not_before = time::OffsetDateTime::from_unix_timestamp(now.as_second())
        .context("Failed to convert start time to OffsetDateTime")?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(future.as_second())
        .context("Failed to convert end time to OffsetDateTime")?;

    params.not_before = not_before;
    params.not_after = not_after;

    let node_cert = params
        .signed_by(&node_key_pair, &lab_ca.cert, &lab_ca.key_pair)
        .context("Failed to sign node certificate with lab CA")?;

    let cert_pem = node_cert.pem();
    write_file_with_permissions(cert_path, cert_pem.as_bytes(), 0o644)
        .context("Failed to write node certificate file")?;

    let key_pem = node_key_pair.serialize_pem();
    write_file_with_permissions(key_path, key_pem.as_bytes(), 0o600)
        .context("Failed to write node private key file")?;

    tracing::info!(
        hostname = %hostname,
        "Node certificate generated: {}",
        cert_path.display()
    );

    Ok(())
}

/// Write a file with specific permissions
pub fn write_file_with_permissions(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;

        // Set directory permissions to 0700
        #[cfg(unix)]
        {
            let metadata = fs::metadata(parent)?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(parent, permissions)?;
        }
    }

    // Write file
    let mut file =
        File::create(path).with_context(|| format!("Failed to create file: {}", path.display()))?;
    file.write_all(content)
        .with_context(|| format!("Failed to write to file: {}", path.display()))?;

    // Set file permissions
    #[cfg(unix)]
    {
        let metadata = file.metadata()?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_self_signed_certificate() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("test.crt");
        let key_path = temp_dir.path().join("test.key");

        let san = vec!["DNS:example.com".to_string(), "IP:192.168.1.1".to_string()];

        let result = generate_self_signed_certificate(&cert_path, &key_path, &san, 365);
        assert!(result.is_ok());

        // Verify files exist
        assert!(cert_path.exists());
        assert!(key_path.exists());

        // Verify key file permissions (Unix only)
        #[cfg(unix)]
        {
            let metadata = fs::metadata(&key_path).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_generate_lab_ca() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("ca.crt");
        let key_path = temp_dir.path().join("ca.key");

        let result = generate_lab_ca(&cert_path, &key_path, "test-lab", 3650);
        assert!(result.is_ok());

        assert!(cert_path.exists());
        assert!(key_path.exists());

        let cert_content = fs::read_to_string(&cert_path).unwrap();
        assert!(cert_content.contains("BEGIN CERTIFICATE"));

        let key_content = fs::read_to_string(&key_path).unwrap();
        assert!(key_content.contains("BEGIN PRIVATE KEY"));

        #[cfg(unix)]
        {
            let metadata = fs::metadata(&key_path).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_generate_node_certificate() {
        let temp_dir = TempDir::new().unwrap();
        let ca_cert_path = temp_dir.path().join("ca.crt");
        let ca_key_path = temp_dir.path().join("ca.key");

        let lab_ca = generate_lab_ca(&ca_cert_path, &ca_key_path, "test-lab", 3650).unwrap();

        let node_cert_path = temp_dir.path().join("node.crt");
        let node_key_path = temp_dir.path().join("node.key");

        let result = generate_node_certificate(
            &node_cert_path,
            &node_key_path,
            &lab_ca,
            "node01",
            "172.31.0.10",
            3650,
        );
        assert!(result.is_ok());

        assert!(node_cert_path.exists());
        assert!(node_key_path.exists());

        let cert_content = fs::read_to_string(&node_cert_path).unwrap();
        assert!(cert_content.contains("BEGIN CERTIFICATE"));

        #[cfg(unix)]
        {
            let metadata = fs::metadata(&node_key_path).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
    }
}
