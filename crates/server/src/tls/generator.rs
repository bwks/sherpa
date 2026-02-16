use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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
            params
                .subject_alt_names
                .push(SanType::DnsName(dns.to_string().try_into().unwrap()));
            tracing::debug!("Added DNS SAN: {}", dns);
        } else {
            // Assume it's a DNS name if no prefix
            params
                .subject_alt_names
                .push(SanType::DnsName(san_entry.to_string().try_into().unwrap()));
            tracing::debug!("Added DNS SAN: {}", san_entry);
        }
    }

    // Add localhost and 127.0.0.1 if not already present
    let has_localhost = san.iter().any(|s| s.contains("localhost"));
    let has_loopback = san.iter().any(|s| s.contains("127.0.0.1"));

    if !has_localhost {
        params.subject_alt_names.push(SanType::DnsName(
            "localhost".to_string().try_into().unwrap(),
        ));
        tracing::debug!("Added default DNS SAN: localhost");
    }

    if !has_loopback {
        params
            .subject_alt_names
            .push(SanType::IpAddress("127.0.0.1".parse().unwrap()));
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

/// Write a file with specific permissions
fn write_file_with_permissions(path: &Path, content: &[u8], mode: u32) -> Result<()> {
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
}
