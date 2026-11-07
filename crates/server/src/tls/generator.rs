use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PKCS_ED25519, SanType,
};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::net::IpAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
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

/// Key algorithm for generated keys and certificates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAlgorithm {
    EcdsaP256,
    Ed25519,
}

/// A certificate authority Sherpa can rebuild from its stored key.
///
/// A certificate it issues only needs the CA's subject and key, so the CA certificate itself
/// does not have to be parsed back.
#[derive(Clone, Copy, Debug)]
pub struct CaSpec<'a> {
    /// Common name (CN) of the CA subject, before the organization.
    pub common_name: Option<&'a str>,
    /// Organization (O) of the CA subject. With no common name either, the subject is empty.
    pub organization: Option<&'a str>,
    pub key_algorithm: KeyAlgorithm,
}

/// Subject of a client certificate.
#[derive(Clone, Copy, Debug)]
pub struct ClientSubject<'a> {
    /// Organization (O); Talos and Kubernetes read the client's role or group from it.
    pub organization: &'a str,
    pub common_name: Option<&'a str>,
}

/// Subject and names of a server certificate.
#[derive(Clone, Copy, Debug)]
pub struct ServerSubject<'a> {
    pub common_name: &'a str,
    pub organization: &'a str,
    /// DNS names and IP addresses the certificate is valid for (SANs).
    pub dns_names: &'a [String],
    pub ip_addresses: &'a [IpAddr],
}

/// A PEM certificate and its PKCS#8 PEM private key.
#[derive(Clone, Debug)]
pub struct PemPair {
    pub cert: String,
    pub key: String,
}

/// Generate a key pair with the given algorithm.
pub fn generate_key_pair(algorithm: KeyAlgorithm) -> Result<KeyPair> {
    let signature = match algorithm {
        KeyAlgorithm::EcdsaP256 => &PKCS_ECDSA_P256_SHA256,
        KeyAlgorithm::Ed25519 => &PKCS_ED25519,
    };
    KeyPair::generate_for(signature).context("Failed to generate key pair")
}

/// Set a certificate's validity to start now and last `validity_days`.
fn set_validity(params: &mut CertificateParams, validity_days: u32) -> Result<()> {
    let now = jiff::Timestamp::now();
    let future = now
        .checked_add(jiff::Span::new().hours(i64::from(validity_days) * 24))
        .context("Failed to calculate certificate expiration date")?;
    params.not_before = time::OffsetDateTime::from_unix_timestamp(now.as_second())
        .context("Failed to convert start time to OffsetDateTime")?;
    params.not_after = time::OffsetDateTime::from_unix_timestamp(future.as_second())
        .context("Failed to convert end time to OffsetDateTime")?;
    Ok(())
}

fn ca_params(spec: &CaSpec, validity_days: u32) -> Result<CertificateParams> {
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create CA certificate parameters")?;
    let mut dn = DistinguishedName::new();
    if let Some(common_name) = spec.common_name {
        dn.push(DnType::CommonName, common_name);
    }
    if let Some(organization) = spec.organization {
        dn.push(DnType::OrganizationName, organization);
    }
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    set_validity(&mut params, validity_days)?;
    Ok(params)
}

/// Generate a self-signed CA in memory.
pub fn generate_ca(spec: &CaSpec, validity_days: u32) -> Result<PemPair> {
    let key_pair = generate_key_pair(spec.key_algorithm)?;
    let cert = ca_params(spec, validity_days)?
        .self_signed(&key_pair)
        .context("Failed to generate CA certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

/// Issue a client-authentication certificate from a CA rebuilt from its PEM key.
pub fn issue_client_certificate(
    ca: &CaSpec,
    ca_key_pem: &str,
    subject: ClientSubject,
    key_algorithm: KeyAlgorithm,
    validity_days: u32,
) -> Result<PemPair> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("Failed to load CA private key")?;
    let ca_cert = ca_params(ca, validity_days)?
        .self_signed(&ca_key)
        .context("Failed to rebuild CA certificate")?;

    let key_pair = generate_key_pair(key_algorithm)?;
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create client certificate parameters")?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, subject.organization);
    if let Some(common_name) = subject.common_name {
        dn.push(DnType::CommonName, common_name);
    }
    params.distinguished_name = dn;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    set_validity(&mut params, validity_days)?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .context("Failed to sign client certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

/// Issue a certificate for a server, also usable as a client certificate, from a CA rebuilt
/// from its PEM key.
pub fn issue_server_certificate(
    ca: &CaSpec,
    ca_key_pem: &str,
    subject: ServerSubject,
    key_algorithm: KeyAlgorithm,
    validity_days: u32,
) -> Result<PemPair> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("Failed to load CA private key")?;
    let ca_cert = ca_params(ca, validity_days)?
        .self_signed(&ca_key)
        .context("Failed to rebuild CA certificate")?;

    let key_pair = generate_key_pair(key_algorithm)?;
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create server certificate parameters")?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, subject.common_name);
    dn.push(DnType::OrganizationName, subject.organization);
    params.distinguished_name = dn;
    for name in subject.dns_names {
        params.subject_alt_names.push(SanType::DnsName(
            name.clone()
                .try_into()
                .with_context(|| format!("Invalid DNS name for a certificate: {name}"))?,
        ));
    }
    for address in subject.ip_addresses {
        params.subject_alt_names.push(SanType::IpAddress(*address));
    }
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    set_validity(&mut params, validity_days)?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .context("Failed to sign server certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

/// Write `content` to `path` with permission bits `mode`, replacing any file there.
///
/// The file never exists with wider permissions: it is written to a temporary file created
/// with `mode` in the same folder, then renamed over `path`. Missing parent folders are created
/// private (0700); existing folders keep their permissions.
pub fn write_file_with_permissions(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("No parent directory for: {}", path.display()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .with_context(|| format!("Failed to create directory: {}", parent.display()))?;

    let file_name = path
        .file_name()
        .with_context(|| format!("No file name in: {}", path.display()))?;
    let temp_path = parent.join(format!(".{}.tmp", file_name.to_string_lossy()));
    // A leftover from an interrupted write would make `create_new` fail.
    match fs::remove_file(&temp_path) {
        Ok(()) => tracing::debug!(path = %temp_path.display(), "Removed a stale temporary file"),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::trace!(path = %temp_path.display(), "No stale temporary file");
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to remove {}", temp_path.display()));
        }
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp_path)
        .with_context(|| format!("Failed to create file: {}", temp_path.display()))?;
    // The umask may have narrowed `mode`; set it exactly.
    file.set_permissions(fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {}", temp_path.display()))?;
    file.write_all(content)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("Failed to write to file: {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to replace file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn writing_a_file_leaves_its_folder_alone() {
        let dir = TempDir::new().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmod");

        write_file_with_permissions(&dir.path().join("key.pem"), b"secret", 0o600).expect("write");

        assert_eq!(mode_of(dir.path()), 0o755);
    }

    #[test]
    fn a_rewritten_file_gets_the_new_mode_and_contents() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("key.pem");
        fs::write(&path, b"old").expect("seed");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");

        write_file_with_permissions(&path, b"secret", 0o600).expect("write");

        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(fs::read(&path).expect("read"), b"secret");
        let leftovers = fs::read_dir(dir.path()).expect("read dir").count();
        assert_eq!(leftovers, 1, "a temporary file was left behind");
    }

    #[test]
    fn missing_folders_are_created_private() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("a/b/cert.pem");

        write_file_with_permissions(&path, b"cert", 0o644).expect("write");

        assert_eq!(mode_of(&path), 0o644);
        assert_eq!(mode_of(&dir.path().join("a")), 0o700);
        assert_eq!(mode_of(&dir.path().join("a/b")), 0o700);
    }

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

    fn parse(pem: &str) -> x509_parser::pem::Pem {
        let (_, pem) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap();
        pem
    }

    #[test]
    fn generated_cas_use_the_requested_key_and_pkcs8() {
        for (algorithm, oid) in [
            (KeyAlgorithm::Ed25519, "1.3.101.112"),
            (KeyAlgorithm::EcdsaP256, "1.2.840.10045.2.1"),
        ] {
            let spec = CaSpec {
                common_name: None,
                organization: Some("talos"),
                key_algorithm: algorithm,
            };
            let ca = generate_ca(&spec, 3650).unwrap();
            assert!(ca.key.starts_with("-----BEGIN PRIVATE KEY-----"));
            let pem = parse(&ca.cert);
            let cert = pem.parse_x509().unwrap();
            assert!(cert.is_ca());
            assert_eq!(cert.subject().to_string(), "O=talos");
            assert_eq!(
                cert.public_key().algorithm.algorithm.to_id_string(),
                oid,
                "{algorithm:?}"
            );
        }
    }

    #[test]
    fn client_certificates_chain_to_the_rebuilt_ca() {
        let spec = CaSpec {
            common_name: None,
            organization: Some("kubernetes"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let ca = generate_ca(&spec, 3650).unwrap();
        let client = issue_client_certificate(
            &spec,
            &ca.key,
            ClientSubject {
                organization: "system:masters",
                common_name: Some("sherpa-admin"),
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let ca_pem = parse(&ca.cert);
        let ca_cert = ca_pem.parse_x509().unwrap();
        let client_pem = parse(&client.cert);
        let client_cert = client_pem.parse_x509().unwrap();
        assert_eq!(client_cert.issuer(), ca_cert.subject());
        assert_eq!(
            client_cert.subject().to_string(),
            "O=system:masters, CN=sherpa-admin"
        );
        assert!(!client_cert.is_ca());
        client_cert
            .verify_signature(Some(ca_cert.public_key()))
            .unwrap();
    }

    #[test]
    fn server_certificates_carry_their_names_and_chain_to_the_rebuilt_ca() {
        let spec = CaSpec {
            common_name: Some("Sherpa Lab CA - lab1"),
            organization: Some("Sherpa"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let ca = generate_ca(&spec, 365).unwrap();
        let dns_names = [
            "vault01".to_owned(),
            "vault01.lab1.sherpa.lab.local".to_owned(),
        ];
        let ip_addresses: [IpAddr; 2] =
            ["172.31.1.21".parse().unwrap(), "fd00::15".parse().unwrap()];
        let server = issue_server_certificate(
            &spec,
            &ca.key,
            ServerSubject {
                common_name: "vault01",
                organization: "Sherpa",
                dns_names: &dns_names,
                ip_addresses: &ip_addresses,
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let ca_pem = parse(&ca.cert);
        let ca_cert = ca_pem.parse_x509().unwrap();
        assert_eq!(
            ca_cert.subject().to_string(),
            "CN=Sherpa Lab CA - lab1, O=Sherpa"
        );
        let server_pem = parse(&server.cert);
        let server_cert = server_pem.parse_x509().unwrap();
        assert_eq!(server_cert.issuer(), ca_cert.subject());
        server_cert
            .verify_signature(Some(ca_cert.public_key()))
            .unwrap();
        let usage = server_cert.extended_key_usage().unwrap().unwrap().value;
        assert!(usage.server_auth && usage.client_auth);
        let names = server_cert
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value;
        let rendered: Vec<String> = names
            .general_names
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(
            rendered,
            [
                "DNSName(vault01)",
                "DNSName(vault01.lab1.sherpa.lab.local)",
                "IPAddress(ac:1f:01:15)",
                "IPAddress(fd:00:00:00:00:00:00:00:00:00:00:00:00:00:00:15)",
            ]
        );
    }

    #[test]
    fn certificates_issued_now_chain_to_lab_cas_made_before_ca_specs() {
        // Labs created before the lab CA moved to `CaSpec` have a CA whose subject is
        // CN then O, with an rcgen default key. Reissuing node certificates from its saved key
        // must produce the same issuer name.
        let key_pair = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![]).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Sherpa Lab CA - old1");
        dn.push(DnType::OrganizationName, "Sherpa");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let old_ca = params.self_signed(&key_pair).unwrap();

        let spec = CaSpec {
            common_name: Some("Sherpa Lab CA - old1"),
            organization: Some("Sherpa"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let server = issue_server_certificate(
            &spec,
            &key_pair.serialize_pem(),
            ServerSubject {
                common_name: "node01",
                organization: "Sherpa",
                dns_names: &["node01".to_owned()],
                ip_addresses: &[],
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let old_pem = old_ca.pem();
        let old_parsed = parse(&old_pem);
        let old_cert = old_parsed.parse_x509().unwrap();
        let server_pem = parse(&server.cert);
        let server_cert = server_pem.parse_x509().unwrap();
        assert_eq!(server_cert.issuer().as_raw(), old_cert.subject().as_raw());
        server_cert
            .verify_signature(Some(old_cert.public_key()))
            .unwrap();
    }
}
